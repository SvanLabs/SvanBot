//! Effective Linux memory capacity and headroom, including enclosing cgroup-v2 limits.
use std::path::Path;

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

pub(super) fn limits(meminfo: &str, group: &Path, root: &Path) -> (u64, u64) {
    let field = |key| {
        meminfo.lines().find_map(|line| line.strip_prefix(key).and_then(|value| value.split_whitespace().next()?.parse::<u64>().ok()))
    };
    let mut capacity = field("MemTotal:").unwrap_or(256 * 1024).saturating_mul(1024);
    let mut available = field("MemAvailable:").unwrap_or(256 * 1024).saturating_mul(1024).min(capacity);
    if group != root && !group.starts_with(root) {
        return (capacity, available);
    }
    let mut path = group;
    loop {
        if let Ok(maximum) = read(&path.join("memory.max")).trim().parse::<u64>() {
            capacity = capacity.min(maximum);
            // A missing usage reading is unknown headroom, not the whole container limit.
            let current = read(&path.join("memory.current")).trim().parse::<u64>().unwrap_or(maximum);
            available = available.min(maximum.saturating_sub(current));
        }
        if path == root {
            break;
        }
        path = path.parent().unwrap_or(root);
    }
    (capacity, available.min(capacity))
}

pub(super) fn detect() -> (u64, u64) {
    let root = Path::new("/sys/fs/cgroup");
    let membership = read(Path::new("/proc/self/cgroup"));
    let group = membership
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .map(|name| Path::new(name.trim_start_matches('/')))
        .filter(|path| !path.components().any(|part| part == std::path::Component::ParentDir))
        .map(|path| root.join(path))
        .unwrap_or_else(|| root.to_path_buf());
    limits(&read(Path::new("/proc/meminfo")), &group, root)
}

pub(super) fn workers(logical: usize, available: u64) -> usize {
    // Conservative default headroom per worker; an explicit compute profile can override it.
    logical.max(1).min((available.saturating_mul(3) / 5 / (256 << 20)).max(1) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enclosing_cgroups_bound_capacity_and_remaining_memory() {
        let root = std::env::temp_dir().join(format!("sv10-memory-limits-{}", std::process::id()));
        let group = root.join("parent/child");
        std::fs::create_dir_all(&group).unwrap();
        std::fs::write(root.join("parent/memory.max"), (1024u64 << 20).to_string()).unwrap();
        std::fs::write(root.join("parent/memory.current"), (768u64 << 20).to_string()).unwrap();
        std::fs::write(group.join("memory.max"), "max").unwrap();
        let observed = limits("MemTotal: 16777216 kB\nMemAvailable: 8388608 kB\n", &group, &root);
        std::fs::remove_dir_all(root).unwrap();
        assert_eq!(observed, (1024 << 20, 256 << 20));
        assert_eq!(workers(32, observed.1), 1);
    }

    #[test]
    fn small_and_unknown_memory_schedule_serially_without_changing_sample_budgets() {
        assert_eq!(workers(32, 0), 1);
        assert_eq!(workers(32, 512 << 20), 1);
        assert_eq!(workers(8, 16 << 30), 8);
        let (capacity, available) = limits("", Path::new("/not-a-group"), Path::new("/elsewhere"));
        assert_eq!((capacity, available), (256 << 20, 256 << 20));
        assert_eq!(workers(32, available), 1);
    }
}
