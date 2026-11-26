//! The checksummed manifest beside a SvanDB log: the bot, how much of the log is committed, and
//! the FNV-1a digest over that committed prefix.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::super::StoreError;

/// The log's format version, in the manifest.
pub(super) const FORMAT: u32 = 1;

/// The FNV-1a 64-bit offset basis.
pub(super) const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// The FNV-1a 64-bit prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// The manifest beside a log: the same name with `.manifest` appended.
pub(super) fn manifest_path(log: &Path) -> PathBuf {
    let mut name = log
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(".manifest");
    log.with_file_name(name)
}

pub(super) fn fnv_update(digest: u64, bytes: &[u8]) -> u64 {
    let mut digest = digest;
    for byte in bytes {
        digest ^= u64::from(*byte);
        digest = digest.wrapping_mul(FNV_PRIME);
    }
    digest
}

/// The manifest: the bot, how much of the log is committed, and its digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Manifest {
    pub(super) format: u32,
    pub(super) bot: String,
    pub(super) records: u64,
    pub(super) bytes: u64,
    pub(super) digest: u64,
}

impl Manifest {
    fn text(&self) -> String {
        json!({
            "format": self.format,
            "bot": self.bot,
            "records": self.records,
            "bytes": self.bytes,
            "digest": format!("{:016x}", self.digest),
        })
        .to_string()
    }

    pub(super) fn read(path: &Path) -> Result<Self, StoreError> {
        let text = fs::read_to_string(path)
            .map_err(|error| StoreError(format!("{}: {error}", path.display())))?;
        let value: Value = serde_json::from_str(&text)
            .map_err(|error| StoreError(format!("{}: {error}", path.display())))?;
        let format = value["format"]
            .as_u64()
            .ok_or_else(|| StoreError("a SvanDB manifest has no format".to_owned()))?;
        if format != u64::from(FORMAT) {
            return Err(StoreError(format!(
                "a SvanDB manifest is format {format}, not {FORMAT}"
            )));
        }
        Ok(Self {
            format: FORMAT,
            bot: value["bot"]
                .as_str()
                .ok_or_else(|| StoreError("a SvanDB manifest has no bot".to_owned()))?
                .to_owned(),
            records: value["records"].as_u64().unwrap_or(0),
            bytes: value["bytes"].as_u64().unwrap_or(0),
            digest: value["digest"]
                .as_str()
                .and_then(|text| u64::from_str_radix(text, 16).ok())
                .ok_or_else(|| StoreError("a SvanDB manifest has no digest".to_owned()))?,
        })
    }

    /// Replaces the manifest with `self`: a temporary file beside it and a rename.
    pub(super) fn write(&self, path: &Path) -> Result<(), StoreError> {
        let temp = path.with_extension("manifest.tmp");
        fs::write(&temp, self.text())
            .map_err(|error| StoreError(format!("{}: {error}", temp.display())))?;
        fs::rename(&temp, path)
            .map_err(|error| StoreError(format!("{}: {error}", path.display())))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_digest_is_fnv_1a() {
        // The standard FNV-1a 64-bit test vectors, and the running property: hashing two halves
        // is hashing the whole.
        assert_eq!(fnv_update(FNV_OFFSET, b""), FNV_OFFSET);
        assert_eq!(fnv_update(FNV_OFFSET, b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv_update(FNV_OFFSET, b"foobar"), 0x8594_4171_f739_67e8);
        let whole = fnv_update(FNV_OFFSET, b"svanbot");
        let halves = fnv_update(fnv_update(FNV_OFFSET, b"svan"), b"bot");
        assert_eq!(whole, halves);
    }

    #[test]
    fn the_manifest_round_trips() {
        let dir = std::env::temp_dir().join(format!("svandb-manifest-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("A.svandb.manifest");
        let manifest = Manifest {
            format: FORMAT,
            bot: "A".into(),
            records: 3,
            bytes: 123,
            digest: 0xdead_beef_cafe_f00d,
        };
        manifest.write(&path).unwrap();
        assert_eq!(Manifest::read(&path).unwrap(), manifest);
    }
}
