//! Host check (0241): the facts about this machine that matter for the fleet, read-only, with the
//! operator command for anything off (the verified checklist in
//! `docs/OPERATIONS.md`). Served at `GET /api/host` for the
//! dashboard's System view, so host drift is visible without a terminal. Nothing here changes the
//! host: every fix needs the operator (and usually sudo).
//!
//! [`read`] gathers the raw files (`/proc`, `/sys`, systemd's timer links, free space); [`checks`]
//! is pure and judges them, so each rule is tested on fixed inputs.

use serde::Serialize;
use std::path::Path;

/// Microcode revision the i7-4770K (Haswell, CPUID family 6 model 60) should load: Debian trixie's
/// `intel-microcode` carries 0x28 (0232).
pub const HASWELL_MICROCODE: u64 = 0x28;
/// Free space on the disk holding `artifacts/` below which the SSD needs attention.
pub const SSD_LOW_BYTES: u64 = 10 << 30;
/// Free space on the archive disk below which the nightly archives and backup mirrors are at risk.
pub const ARCHIVE_LOW_BYTES: u64 = 20 << 30;
/// Available memory below which the fleet, learner and analyst would be squeezed.
pub const MEMORY_LOW_BYTES: u64 = 2 << 30;
/// Swap held by our *own* processes above which the fleet can feel it: a page of ours that has to
/// come back in is latency inside a decision. An idle SvanBot process holds a few MB, far below
/// this, so it only trips on real pressure; other programs' swap costs us nothing.
pub const OUR_SWAP_LOW_BYTES: u64 = 256 << 20;
/// Our binaries, for spotting a fleet process by name when its path does not say where it came from.
const OUR_BINARIES: [&str; 4] = ["sv10-bot", "learner", "analyst", "sv10-nightly"];

/// How a fact reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// As recommended.
    Ok,
    /// Worth the operator's attention; `advice` says what to run.
    Warn,
    /// Shown for context; nothing to do.
    Info,
}

/// One row of the panel.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Check {
    /// Stable id.
    pub key: &'static str,
    /// What is checked.
    pub label: &'static str,
    /// What was found.
    pub value: String,
    /// Verdict.
    pub status: Status,
    /// The operator's fix when `status` is `Warn`.
    pub advice: Option<String>,
}

/// Raw host facts (each `None` when unreadable, e.g. inside a container).
#[derive(Clone, Debug, Default)]
pub struct Sources {
    /// `/proc/cpuinfo`.
    pub cpuinfo: String,
    /// `/proc/meminfo`.
    pub meminfo: String,
    /// `/sys/kernel/mm/transparent_hugepage/enabled`.
    pub thp: Option<String>,
    /// cpu0's `scaling_driver` and `scaling_governor`.
    pub scaling: Option<(String, String)>,
    /// Whether `fstrim.timer` is enabled (a `timers.target.wants` link exists), if systemd is there.
    pub fstrim_enabled: Option<bool>,
    /// `/proc/sys/kernel/osrelease`.
    pub kernel: Option<String>,
    /// CPU temperature and thermal throttling (`coretemp`, `thermal_throttle`), when the sensors exist.
    pub heat: Option<Heat>,
    /// Free bytes on the disk holding `artifacts/`.
    pub ssd_free: Option<u64>,
    /// Free bytes on the archive directory's disk, whether that is another disk than `artifacts/`,
    /// and the directory itself (named in the same-disk advice, #725).
    pub archive: Option<(u64, bool, std::path::PathBuf)>,
    /// Every mounted ext4 filesystem's error record (`/sys/fs/ext4/<dev>/`).
    pub filesystems: Vec<FsHealth>,
    /// `/proc/pressure/memory` `full avg300`, percent.
    pub memory_pressure: Option<f64>,
    /// What our own processes hold in swap, when any of them do (0304).
    pub our_swap: Option<OurSwap>,
    /// Unix seconds of the reading (to age the last filesystem error).
    pub now: u64,
}

/// Swap held by svanbot10's own processes. The machine's swap can be full of other programs' idle
/// pages and cost the fleet nothing; our own pages in swap are the ones that cost time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OurSwap {
    /// `(name, bytes)` per swapped process, largest first.
    pub processes: Vec<(String, u64)>,
}

impl OurSwap {
    /// All of it.
    pub fn total(&self) -> u64 {
        self.processes.iter().map(|(_, b)| b).sum()
    }

    /// The largest holder, as it reads in the panel.
    fn worst(&self) -> Option<String> {
        self.processes.first().map(|(n, b)| format!("{n} {}", size(*b)))
    }
}

/// Which processes are ours: the fleet and the tools, by where they run from or by binary name.
fn is_ours(exe: &str, cmdline: &str) -> bool {
    let named = |s: &str| OUR_BINARIES.iter().any(|b| s.ends_with(b));
    exe.contains("svanbot10") || named(exe) || cmdline.split_whitespace().next().is_some_and(named)
}

/// Read every process's `VmSwap` from `/proc` and keep ours (0304). Unreadable processes (exited
/// between the listing and the read, or not ours to look at) are skipped: this only reports.
/// `Some` with nothing in it means our processes were found and none of them is in swap — the row
/// says "none", which is a different fact from having nothing to say.
fn read_our_swap() -> Option<OurSwap> {
    let mut processes: Vec<(String, u64)> = Vec::new();
    let mut found = false;
    for entry in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
        let pid = entry.file_name();
        if !pid.to_str().is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) {
            continue;
        }
        let dir = entry.path();
        let exe = std::fs::read_link(dir.join("exe")).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        let cmdline = std::fs::read(dir.join("cmdline")).map(|b| String::from_utf8_lossy(&b).replace('\0', " ")).unwrap_or_default();
        if !is_ours(&exe, &cmdline) {
            continue;
        }
        found = true;
        let status = std::fs::read_to_string(dir.join("status")).unwrap_or_default();
        let Some(bytes) = meminfo_bytes(&status, "VmSwap").filter(|b| *b > 0) else { continue };
        let name = exe.rsplit('/').next().unwrap_or("?").to_string();
        processes.push((format!("{name} (pid {})", pid.to_string_lossy()), bytes));
    }
    processes.sort_by_key(|(_, bytes)| std::cmp::Reverse(*bytes));
    found.then_some(OurSwap { processes })
}

mod disks;
mod thermal;
use disks::read_filesystems;
pub use disks::{DiskHealth, FsHealth, parent_disk, parse_smart};
pub use thermal::Heat;

/// Read the host's files (never fails: an unreadable fact is left out of the verdicts).
pub fn read(artifacts: &Path, archive_dir: &Path) -> Sources {
    use std::os::unix::fs::MetadataExt;
    let text = |p: &str| std::fs::read_to_string(p).ok().map(|s| s.trim().to_string());
    let scaling =
        text("/sys/devices/system/cpu/cpu0/cpufreq/scaling_driver").zip(text("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"));
    let systemd = Path::new("/etc/systemd/system").is_dir();
    let fstrim_enabled = systemd.then(|| Path::new("/etc/systemd/system/timers.target.wants/fstrim.timer").exists());
    let dev = |p: &Path| std::fs::metadata(p).ok().map(|m| m.dev());
    let archive = sv10_rt::free_bytes(archive_dir)
        .map(|free| (free, dev(archive_dir).is_some() && dev(archive_dir) != dev(artifacts), archive_dir.to_path_buf()));
    Sources {
        cpuinfo: text("/proc/cpuinfo").unwrap_or_default(),
        meminfo: text("/proc/meminfo").unwrap_or_default(),
        thp: text("/sys/kernel/mm/transparent_hugepage/enabled"),
        scaling,
        fstrim_enabled,
        kernel: text("/proc/sys/kernel/osrelease"),
        heat: thermal::read(),
        ssd_free: sv10_rt::free_bytes(artifacts),
        archive,
        filesystems: read_filesystems(),
        memory_pressure: text("/proc/pressure/memory").and_then(|p| {
            p.lines().find(|l| l.starts_with("full"))?.split_whitespace().find_map(|f| f.strip_prefix("avg300="))?.parse().ok()
        }),
        our_swap: read_our_swap(),
        now: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
    }
}

/// The first value of `key` in `/proc/cpuinfo`-style `key : value` text.
fn field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        (k.trim() == key).then(|| v.trim())
    })
}

/// A `/proc/meminfo` entry in bytes.
fn meminfo_bytes(text: &str, key: &str) -> Option<u64> {
    field(text, key)?.split_whitespace().next()?.parse::<u64>().ok().map(|kb| kb * 1024)
}

fn gib(b: u64) -> String {
    format!("{:.1} GiB", b as f64 / (1u64 << 30) as f64)
}

/// Small amounts in MiB: our own swap is megabytes, and "0.0 GiB" would hide exactly the fact the
/// row exists to show (0304).
fn size(b: u64) -> String {
    if b < (1 << 30) { format!("{} MB", b >> 20) } else { gib(b) }
}

/// Judge the facts.
pub fn checks(src: &Sources) -> Vec<Check> {
    let mut out = Vec::new();
    let mut push = |key, label, value: String, status, advice: Option<&str>| {
        out.push(Check { key, label, value, status, advice: advice.map(String::from) })
    };

    let model = field(&src.cpuinfo, "model name").unwrap_or("unknown").to_string();
    let avx2 = field(&src.cpuinfo, "flags").is_some_and(|f| f.split_whitespace().any(|x| x == "avx2"));
    push("cpu", "CPU", format!("{model}{}", if avx2 { " · AVX2" } else { "" }), Status::Info, None);

    // Microcode: judged on the target CPU only (family 6, model 60); elsewhere shown for context.
    let microcode = field(&src.cpuinfo, "microcode").and_then(|m| u64::from_str_radix(m.trim_start_matches("0x"), 16).ok());
    let haswell = field(&src.cpuinfo, "cpu family") == Some("6") && field(&src.cpuinfo, "model") == Some("60");
    if let Some(rev) = microcode {
        let (status, advice) = if haswell && rev < HASWELL_MICROCODE {
            (Status::Warn, Some("sudo apt install intel-microcode (enable non-free-firmware in the APT sources), then reboot"))
        } else if haswell {
            (Status::Ok, None)
        } else {
            (Status::Info, None)
        };
        push(
            "microcode",
            "CPU microcode",
            format!("0x{rev:x}{}", if haswell { format!(" (current 0x{HASWELL_MICROCODE:x})") } else { String::new() }),
            status,
            advice,
        );
    }

    if let Some((driver, governor)) = &src.scaling {
        push("scaling", "Frequency scaling", format!("{driver} · {governor}"), Status::Info, None);
    }

    if let Some(thp) = &src.thp {
        // The bracketed word is the active mode.
        let mode =
            thp.split_whitespace().find(|w| w.starts_with('[')).map(|w| w.trim_matches(['[', ']'])).unwrap_or(thp.as_str()).to_string();
        let (status, advice) = match mode.as_str() {
            "always" | "madvise" => (Status::Ok, None),
            _ => (
                Status::Warn,
                Some("echo always | sudo tee /sys/kernel/mm/transparent_hugepage/enabled (0069: the strength tables use huge pages)"),
            ),
        };
        push("thp", "Transparent huge pages", mode, status, advice);
    }

    if let (Some(avail), Some(total)) = (meminfo_bytes(&src.meminfo, "MemAvailable"), meminfo_bytes(&src.meminfo, "MemTotal")) {
        let (status, advice) = if avail < MEMORY_LOW_BYTES {
            (Status::Warn, Some("close other programs, or lower SVANBOT_LIVE_SAMPLES / learner threads until memory frees"))
        } else {
            (Status::Ok, None)
        };
        push("memory", "Memory available", format!("{} of {}", gib(avail), gib(total)), status, advice);
    }
    if let (Some(swap_total), Some(swap_free)) = (meminfo_bytes(&src.meminfo, "SwapTotal"), meminfo_bytes(&src.meminfo, "SwapFree"))
        && swap_total > 0
    {
        // Full swap alone costs nothing (idle pages of other programs); it matters when *our* pages
        // are in it or when the kernel stalls on memory, which is what the fleet would feel (0304).
        let ours = src.our_swap.as_ref();
        let ours_swapped = ours.is_some_and(|o| o.total() > OUR_SWAP_LOW_BYTES);
        let pressured = src.memory_pressure.is_some_and(|p| p >= 1.0);
        // "ours none" and "nothing read" are different facts: the first is good news, the second is
        // a missing fact, and the row must not read the same for both (0304).
        let ours_text = ours.map_or(String::new(), |o| {
            if o.processes.is_empty() {
                return " · ours none".to_string();
            }
            let worst = o.worst().map_or(String::new(), |w| format!(" ({w})"));
            format!(" · ours {}{worst}", size(o.total()))
        });
        let pressure = src.memory_pressure.map_or(String::new(), |p| format!(" · memory pressure {p:.1}%"));
        let advice = if ours_swapped {
            Some(
                "our own pages are in swap and cost the fleet on the way back in: check the holders with grep VmSwap /proc/*/status | sort -k2 -n | tail, free memory (close idle services), or sudo swapoff -a && sudo swapon -a while the fleet is quiet",
            )
        } else if pressured {
            Some(
                "the kernel is stalling on memory: find the swapped processes with grep VmSwap /proc/*/status | sort -k2 -n | tail; stop idle services, or sudo swapoff -a && sudo swapon -a once memory is free",
            )
        } else {
            None
        };
        push(
            "swap",
            "Swap in use",
            format!("{} of {}{ours_text}{pressure}", gib(swap_total - swap_free), gib(swap_total)),
            if ours_swapped || pressured { Status::Warn } else { Status::Info },
            advice,
        );
    }

    if let Some(free) = src.ssd_free {
        let (status, advice) = if free < SSD_LOW_BYTES {
            (
                Status::Warn,
                Some(
                    "free space on the SSD: ./target/release/review storage, prune artifacts/release-snapshots, keep backups on the HDD (SVANBOT_ARCHIVE_DIR)",
                ),
            )
        } else {
            (Status::Ok, None)
        };
        push("ssd", "Free space (databases' disk)", gib(free), status, advice);
    }
    match src.archive {
        Some((free, true, _)) => {
            let (status, advice) = if free < ARCHIVE_LOW_BYTES {
                (Status::Warn, Some("prune old archives: ./target/release/archive list, then remove the oldest one-off copies"))
            } else {
                (Status::Ok, None)
            };
            push("archive", "Free space (archive disk)", gib(free), status, advice);
        }
        Some((free, false, ref dir)) => {
            // A dedicated folder on the databases' disk is the honest fallback on a one-disk box
            // (#725): it survives deletion and rotation mistakes, not losing the disk. The advice
            // names the folder and the only thing that closes the disk-loss gap — a second disk or a
            // network share — instead of a path that cannot exist here.
            let advice = format!(
                "the nightly archives (and the hourly mirror, when SVANBOT_MIRROR_HOURLY_BACKUPS is on) go to a dedicated folder on this disk ({}): \
                 that guards against deletion and rotation mistakes, not losing the disk. Only a second disk or a network share closes that gap; \
                 attach one and point SVANBOT_ARCHIVE_DIR at it",
                dir.display()
            );
            push("archive", "Archive disk", format!("same disk as the databases ({} free)", gib(free)), Status::Warn, Some(&advice));
        }
        None => {}
    }

    let failing: Vec<String> = src
        .filesystems
        .iter()
        .filter(|f| f.errors > 0)
        .map(|f| {
            let age = if f.last_error_time > 0 {
                format!(", latest {} min ago", src.now.saturating_sub(f.last_error_time) / 60)
            } else {
                String::new()
            };
            // ext4's own error numbering (`EXT4_ERR_*` in fs/ext4/super.c), not errno.
            let code = match f.last_errcode {
                Some(2) => " (I/O error)".to_string(),
                Some(4) => " (bad checksum)".to_string(),
                Some(5) => " (corrupted metadata)".to_string(),
                Some(c) if c > 0 => format!(" (ext4 error {c})"),
                _ => String::new(),
            };
            let smart = match &f.disk {
                Some((_, d)) if d.failing || d.bad_sectors > 0 => {
                    format!("; disk SMART: {} bad sectors{}", d.bad_sectors, if d.failing { ", failing" } else { "" })
                }
                Some(_) => "; disk healthy (SMART: not failing, 0 bad sectors)".to_string(),
                None => String::new(),
            };
            format!("{} ({}): {} errors{code}{age}{smart}", f.mount.as_deref().unwrap_or("unmounted"), f.device, f.errors)
        })
        .collect();
    // The advice follows the worst filesystem: hardware trouble (SMART, or an I/O error from the
    // device) is a replacement; damage on a disk that reports itself healthy is a repair (0307).
    let advice = |f: &FsHealth| {
        let mount = f.mount.as_deref().unwrap_or("<mount>");
        let disk = f.disk.as_ref().map_or_else(|| parent_disk(&f.device), |(d, _)| d.clone());
        let io_error = [f.first_errcode, f.last_errcode].contains(&Some(2));
        match &f.disk {
            Some((_, d)) if d.failing || d.bad_sectors > 0 || io_error => format!(
                "the disk reports hardware trouble: sudo smartctl -a /dev/{disk}; copy anything irreplaceable off it now; sudo umount {mount} && sudo e2fsck -f /dev/{}; replace the disk",
                f.device
            ),
            None if io_error => format!(
                "the device returned I/O errors: sudo smartctl -a /dev/{disk}; copy anything irreplaceable off it now; sudo umount {mount} && sudo e2fsck -f /dev/{}; replace the disk if SMART shows bad sectors",
                f.device
            ),
            Some(_) => format!(
                "filesystem damage on a healthy disk (no I/O errors, SMART clean): writes that need a damaged block group fail until it is repaired. Stop what writes to it, then sudo umount {mount} && sudo e2fsck -f /dev/{} && sudo mount {mount}; no new disk needed",
                f.device
            ),
            None => format!(
                "the filesystem is damaged; SMART was not readable here, so check the disk with sudo smartctl -a /dev/{disk}, then sudo umount {mount} && sudo e2fsck -f /dev/{} && sudo mount {mount}",
                f.device
            ),
        }
    };
    let worst = src.filesystems.iter().filter(|f| f.errors > 0).max_by_key(|f| {
        let hardware =
            f.disk.as_ref().is_some_and(|(_, d)| d.failing || d.bad_sectors > 0) || [f.first_errcode, f.last_errcode].contains(&Some(2));
        (hardware, f.errors)
    });
    if !src.filesystems.is_empty() {
        if failing.is_empty() {
            push("filesystems", "Filesystem errors", "none".to_string(), Status::Ok, None);
        } else {
            let advice = worst.map(advice);
            push("filesystems", "Filesystem errors", failing.join("; "), Status::Warn, advice.as_deref());
        }
    }

    if let Some(enabled) = src.fstrim_enabled {
        let (status, advice) = if enabled {
            (Status::Ok, None)
        } else {
            (Status::Warn, Some("sudo systemctl enable --now fstrim.timer (weekly TRIM for the DRAM-less SSD)"))
        };
        push("fstrim", "SSD TRIM timer", if enabled { "enabled" } else { "not enabled" }.to_string(), status, advice);
    }

    if let Some(h) = &src.heat {
        let (value, hot) = thermal::verdict(h);
        let advice = "the CPU reaches its 100 °C limit and slows itself down. Expected while builds, tests and the learner run flat out, and \
                      harmless to the chip, but throttled time is lost work: a cleaner cooler (fins, fan curve, paste) or fewer learner threads \
                      (a balanced or custom compute profile on the dashboard) gives it back";
        push("heat", "CPU heat", value, if hot { Status::Warn } else { Status::Ok }, hot.then_some(advice));
    }

    if let Some(kernel) = &src.kernel {
        push("kernel", "Kernel", kernel.clone(), Status::Info, None);
    }
    out
}

#[cfg(test)]
mod tests;
