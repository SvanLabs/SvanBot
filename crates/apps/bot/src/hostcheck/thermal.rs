//! The host check's heat probe (2026-09-27): the hottest `coretemp` sensor now and the time the CPU
//! package has spent thermally throttled since boot. The i7-4770K sat at 90 °C with 7 h of throttling
//! in 11.5 days and nothing on the dashboard said so.

/// What the CPU's sensors say.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Heat {
    /// The hottest `coretemp` reading now, °C.
    pub max_c: f64,
    /// `package_throttle_total_time_ms` of cpu0: time the package ran throttled since boot.
    pub throttled_ms: u64,
    /// Seconds since boot.
    pub uptime_s: u64,
}

/// Temperature at which the row warns (Haswell throttles at 100 °C).
const HOT_C: f64 = 90.0;
/// Share of uptime spent throttled at which the row warns.
const THROTTLED_SHARE: f64 = 0.01;

/// The row's text and whether it warns.
pub(super) fn verdict(h: &Heat) -> (String, bool) {
    let share = h.throttled_ms as f64 / 1000.0 / h.uptime_s.max(1) as f64;
    let value = format!(
        "{:.0} °C now; thermal throttling {:.1} h since boot ({:.1}% of uptime)",
        h.max_c,
        h.throttled_ms as f64 / 3.6e6,
        share * 100.0
    );
    (value, h.max_c >= HOT_C || share >= THROTTLED_SHARE)
}

/// Read the sensors; `None` without a `coretemp` driver.
pub(super) fn read() -> Option<Heat> {
    let num = |p: std::path::PathBuf| std::fs::read_to_string(p).ok()?.trim().parse::<u64>().ok();
    let coretemp = std::fs::read_dir("/sys/class/hwmon")
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| std::fs::read_to_string(p.join("name")).is_ok_and(|n| n.trim() == "coretemp"))?;
    let max_milli = std::fs::read_dir(&coretemp)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().ends_with("_input")))
        .filter_map(num)
        .max()?;
    let throttled_ms = num("/sys/devices/system/cpu/cpu0/thermal_throttle/package_throttle_total_time_ms".into()).unwrap_or(0);
    let uptime_s = std::fs::read_to_string("/proc/uptime").ok()?.split_whitespace().next()?.parse::<f64>().ok()? as u64;
    Some(Heat { max_c: max_milli as f64 / 1000.0, throttled_ms, uptime_s })
}
