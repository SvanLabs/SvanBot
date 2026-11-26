//! SvanDB (V11-128): a prototype append-only store behind the [`Store`] API surface.
//!
//! One log file a bot, one JSON record a line, and a checksummed manifest that says how many
//! records and bytes are committed and what the log's FNV-1a digest is over them. An append
//! writes the line, then rewrites the manifest atomically (a temporary file and a rename), then
//! returns. A kill between the two writes leaves a line past the manifest's committed length: it
//! was never committed, and the next open truncates it. The in-memory index is rebuilt from the
//! committed prefix at open, so a read is a scan of the log once and a lookup after that.
//!
//! This is a prototype for the measured gate on V11-124, not the fleet's store: it is not wired
//! into `main`, and `Store` stays SQLite. It answers the same methods, in the same order, so the
//! gate can measure it on the fleet's own workload ([`Svandb::import`] copies a SQLite store in).

use std::cell::RefCell;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{StoreError, Totals};

mod index;
mod manifest;
mod reads;
#[cfg(test)]
mod tests;
mod writes;

use index::{Index, apply};
use manifest::{FNV_OFFSET, FORMAT, Manifest, fnv_update, manifest_path};

/// The writable end of a log: where the next record goes, and what is committed.
#[derive(Debug)]
pub(super) struct Journal {
    pub(super) file: File,
    pub(super) bytes: u64,
    pub(super) digest: u64,
    pub(super) records: u64,
}

/// One bot's SvanDB log.
#[derive(Debug)]
pub struct Svandb {
    path: PathBuf,
    manifest: PathBuf,
    bot: String,
    read_only: bool,
    journal: RefCell<Journal>,
    index: RefCell<Index>,
}

impl Svandb {
    /// Opens `path`'s log for `bot`, creating it if it is new. A log made for another bot is
    /// refused, as [`super::Store::open`] refuses one.
    pub fn open(path: &Path, bot: &str) -> Result<Self, StoreError> {
        if path.exists() || manifest_path(path).exists() {
            Self::load(path, Some(bot), false)
        } else {
            Self::create(path, bot)
        }
    }

    /// Opens `path`'s log to read only: the dashboard's handle, as [`super::Store::open_reader`]
    /// is.
    pub fn open_reader(path: &Path) -> Result<Self, StoreError> {
        Self::load(path, None, true)
    }

    fn create(path: &Path, bot: &str) -> Result<Self, StoreError> {
        let manifest = manifest_path(path);
        let file = File::create(path)
            .map_err(|error| StoreError(format!("{}: {error}", path.display())))?;
        Manifest {
            format: FORMAT,
            bot: bot.to_owned(),
            records: 0,
            bytes: 0,
            digest: FNV_OFFSET,
        }
        .write(&manifest)?;
        Ok(Self {
            path: path.to_owned(),
            manifest,
            bot: bot.to_owned(),
            read_only: false,
            journal: RefCell::new(Journal {
                file,
                bytes: 0,
                digest: FNV_OFFSET,
                records: 0,
            }),
            index: RefCell::new(Index::default()),
        })
    }

    fn load(path: &Path, expected_bot: Option<&str>, read_only: bool) -> Result<Self, StoreError> {
        let manifest_file = manifest_path(path);
        let manifest = Manifest::read(&manifest_file)?;
        if let Some(bot) = expected_bot
            && manifest.bot != bot
        {
            return Err(StoreError(format!(
                "this store is {}'s; {bot} may not open it",
                manifest.bot
            )));
        }
        let file = if read_only {
            File::open(path).map_err(|error| StoreError(format!("{}: {error}", path.display())))?
        } else {
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .map_err(|error| StoreError(format!("{}: {error}", path.display())))?
        };
        let len = file
            .metadata()
            .map_err(|error| StoreError(error.to_string()))?
            .len();
        if len < manifest.bytes {
            return Err(StoreError(format!(
                "{} is {len} bytes, under the manifest's committed {}",
                path.display(),
                manifest.bytes
            )));
        }
        // A kill between the log write and the manifest rewrite leaves an uncommitted tail: it
        // was never acknowledged, so the writer drops it and the reader never looks at it.
        if len > manifest.bytes && !read_only {
            file.set_len(manifest.bytes)
                .map_err(|error| StoreError(error.to_string()))?;
        }
        let mut committed = vec![0; usize::try_from(manifest.bytes).unwrap_or(usize::MAX)];
        {
            let mut reader = &file;
            reader
                .seek(SeekFrom::Start(0))
                .map_err(|error| StoreError(error.to_string()))?;
            reader
                .read_exact(&mut committed)
                .map_err(|error| StoreError(error.to_string()))?;
        }
        if fnv_update(FNV_OFFSET, &committed) != manifest.digest {
            return Err(StoreError(format!(
                "{}'s manifest digest does not match its log",
                path.display()
            )));
        }
        let mut index = Index::default();
        let mut records = 0u64;
        if !committed.is_empty() {
            if committed.last() != Some(&b'\n') {
                return Err(StoreError(format!(
                    "{}'s committed log does not end at a record",
                    path.display()
                )));
            }
            for line in committed.split_inclusive(|byte| *byte == b'\n') {
                let text = std::str::from_utf8(line)
                    .map_err(|_| StoreError("a SvanDB record is not UTF-8".to_owned()))?
                    .trim_end_matches('\n');
                let value: Value = serde_json::from_str(text)
                    .map_err(|error| StoreError(format!("a SvanDB record: {error}")))?;
                apply(&mut index, &value).map_err(StoreError)?;
                records += 1;
            }
        }
        if records != manifest.records {
            return Err(StoreError(format!(
                "{}'s manifest counts {} records, its log has {records}",
                path.display(),
                manifest.records
            )));
        }
        Ok(Self {
            path: path.to_owned(),
            manifest: manifest_file,
            bot: manifest.bot.clone(),
            read_only,
            journal: RefCell::new(Journal {
                file,
                bytes: manifest.bytes,
                digest: manifest.digest,
                records: manifest.records,
            }),
            index: RefCell::new(index),
        })
    }

    /// The log a caller can point at, for the benchmark and the operator.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The log's format name: SvanDB has no journal mode, and says so rather than pretending.
    pub fn journal_mode(&self) -> Result<String, StoreError> {
        Ok("append-only".to_owned())
    }

    /// The bot this log belongs to.
    pub fn bot(&self) -> Result<String, StoreError> {
        Ok(self.bot.clone())
    }

    /// How many hands and incidents the log holds, and when the newest hand was written.
    pub fn totals(&self) -> Result<Totals, StoreError> {
        let index = self.index.borrow();
        Ok(Totals {
            hands: u64::try_from(index.hands.len()).unwrap_or(u64::MAX),
            incidents: u64::try_from(index.incidents.len()).unwrap_or(u64::MAX),
            last_hand_ms: index.hands.iter().map(|hand| hand.recorded_ms).max(),
        })
    }

    /// Writes one record and commits it: the line, then the manifest. The caller applies it to
    /// the index, so the on-disk prefix and the index never disagree after a successful append.
    pub(super) fn append(&self, record: Value) -> Result<(), StoreError> {
        if self.read_only {
            return Err(StoreError("this store is open for reading only".to_owned()));
        }
        let line = format!("{record}\n");
        let mut journal = self.journal.borrow_mut();
        journal
            .file
            .seek(SeekFrom::End(0))
            .map_err(|error| StoreError(error.to_string()))?;
        journal
            .file
            .write_all(line.as_bytes())
            .map_err(|error| StoreError(error.to_string()))?;
        let bytes = journal.bytes + u64::try_from(line.len()).unwrap_or(0);
        let digest = fnv_update(journal.digest, line.as_bytes());
        let records = journal.records + 1;
        Manifest {
            format: FORMAT,
            bot: self.bot.clone(),
            records,
            bytes,
            digest,
        }
        .write(&self.manifest)?;
        journal.bytes = bytes;
        journal.digest = digest;
        journal.records = records;
        apply(&mut self.index.borrow_mut(), &record).map_err(StoreError)?;
        Ok(())
    }
}
