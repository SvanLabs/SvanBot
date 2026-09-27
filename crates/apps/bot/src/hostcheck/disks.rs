//! The host check's disk probe (0307, split out of `hostcheck.rs`): every mounted ext4 filesystem's
//! error record, and for one with errors, the disk's own SMART summary through udisks.

use std::path::Path;

/// One ext4 filesystem's error record. The kernel counts every error it hit on the filesystem
/// (`EIO` reading a block bitmap, a failed journal write); any at all means the filesystem is
/// damaged, and writes to it may be lost after they were reported done (0308). Whether the *disk*
/// is failing is a separate question, answered by its own SMART record and by whether any error was
/// an I/O error (2026-09-27: 37 errors on a disk whose SMART was spotless, 0307).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FsHealth {
    /// Block device name, e.g. `sdb1`.
    pub device: String,
    /// Where it is mounted, when known.
    pub mount: Option<String>,
    /// `errors_count`.
    pub errors: u64,
    /// `last_error_time`, unix seconds (0 = none).
    pub last_error_time: u64,
    /// `last_error_errcode`, ext4's own code (2 I/O error, 4 bad checksum, 5 corrupted metadata).
    pub last_errcode: Option<i64>,
    /// `first_error_errcode`, on the same scale.
    pub first_errcode: Option<i64>,
    /// The disk under it and its SMART summary, when udisks could say (no root needed).
    pub disk: Option<(String, DiskHealth)>,
}

/// A disk's own verdict on itself, from udisks' SMART summary.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiskHealth {
    /// `SmartFailing`: an attribute is past its threshold.
    pub failing: bool,
    /// `SmartNumBadSectors`: reallocated plus pending sectors.
    pub bad_sectors: u64,
}

/// The disk a partition belongs to: `sdb1` → `sdb`, `nvme0n1p2` → `nvme0n1`, `mmcblk0p1` → `mmcblk0`.
pub fn parent_disk(device: &str) -> String {
    let trimmed = device.trim_end_matches(|c: char| c.is_ascii_digit());
    match trimmed.strip_suffix('p') {
        // `nvme0n1p2`, `mmcblk0p1`: the partition number follows a `p` after the disk's own digit.
        Some(disk) if disk.ends_with(|c: char| c.is_ascii_digit()) => disk.to_string(),
        _ if trimmed.ends_with(|c: char| c.is_ascii_digit()) || trimmed.is_empty() => device.to_string(),
        _ => trimmed.to_string(),
    }
}

/// `udisksctl info` text for a drive → its SMART summary; `None` without SMART support.
pub fn parse_smart(text: &str) -> Option<DiskHealth> {
    let field = |name: &str| text.lines().find_map(|l| l.trim().strip_prefix(name).map(|v| v.trim().to_string()));
    if field("SmartSupported:")? != "true" {
        return None;
    }
    Some(DiskHealth { failing: field("SmartFailing:")? == "true", bad_sectors: field("SmartNumBadSectors:")?.parse().ok()? })
}

/// The SMART summary of `disk` through udisks, which serves it to any user; `None` when udisks is
/// missing or says nothing (then the check makes no claim about the hardware).
fn read_smart(disk: &str) -> Option<DiskHealth> {
    let run = |args: &[&str]| {
        std::process::Command::new("udisksctl")
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    let block = run(&["info", "-b", &format!("/dev/{disk}")])?;
    let drive = block.lines().find_map(|l| l.trim().strip_prefix("Drive:"))?.trim().trim_matches('\'').to_string();
    parse_smart(&run(&["info", "-p", drive.strip_prefix("/org/freedesktop/UDisks2/")?])?)
}

/// The ext4 error records of every mounted ext4 filesystem.
pub(super) fn read_filesystems() -> Vec<FsHealth> {
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    let mount_of = |dev: &str| {
        mounts.lines().find_map(|l| {
            let mut f = l.split_whitespace();
            let (source, target) = (f.next()?, f.next()?);
            (source == format!("/dev/{dev}")).then(|| target.to_string())
        })
    };
    let num = |dir: &Path, name: &str| std::fs::read_to_string(dir.join(name)).ok().and_then(|v| v.trim().parse::<i64>().ok());
    let mut out: Vec<FsHealth> = std::fs::read_dir("/sys/fs/ext4")
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let dir = e.path();
            let errors = num(&dir, "errors_count")?;
            let device = e.file_name().to_string_lossy().into_owned();
            // SMART only where it matters: a filesystem with errors (udisks is a process call).
            let disk = (errors > 0).then(|| parent_disk(&device)).and_then(|d| read_smart(&d).map(|h| (d, h)));
            Some(FsHealth {
                mount: mount_of(&device),
                device,
                errors: errors.max(0) as u64,
                last_error_time: num(&dir, "last_error_time").unwrap_or(0).max(0) as u64,
                last_errcode: num(&dir, "last_error_errcode"),
                first_errcode: num(&dir, "first_error_errcode"),
                disk,
            })
        })
        .collect();
    out.sort_by(|a, b| a.device.cmp(&b.device));
    out
}
