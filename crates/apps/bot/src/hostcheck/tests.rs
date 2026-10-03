//! Tests for the host check: what each row reports and what it says when it cannot tell.
//!
//! Split out of `hostcheck.rs` (0320: the 500-line rule).

use super::*;

/// A fixture archive folder for the one-disk case (#725); the advice must name it.
const SAME_DISK_ARCHIVE: &str = "/srv/svanbot10/backups/archive";

fn haswell(microcode: &str) -> String {
    format!(
        "processor\t: 0\nvendor_id\t: GenuineIntel\ncpu family\t: 6\nmodel\t\t: 60\nmodel name\t: Intel(R) Core(TM) i7-4770K CPU @ 3.50GHz\nmicrocode\t: {microcode}\nflags\t\t: fpu sse4_2 avx avx2 bmi2\n"
    )
}

fn meminfo(avail_kb: u64, swap_free_kb: u64) -> String {
    format!("MemTotal:       32843936 kB\nMemAvailable:   {avail_kb} kB\nSwapTotal:       6082556 kB\nSwapFree:        {swap_free_kb} kB\n")
}

fn get<'a>(checks: &'a [Check], key: &str) -> &'a Check {
    checks.iter().find(|c| c.key == key).unwrap_or_else(|| panic!("no {key} check in {checks:?}"))
}

#[test]
fn the_target_box_as_measured_needs_microcode_and_nothing_else() {
    let src = Sources {
        cpuinfo: haswell("0x27"),
        meminfo: meminfo(22_000_000, 1_600_000),
        thp: Some("[always] madvise never".into()),
        scaling: Some(("intel_pstate".into(), "powersave".into())),
        fstrim_enabled: Some(true),
        kernel: Some("6.12.107".into()),
        ssd_free: Some(25 << 30),
        archive: Some((136 << 30, true, "/backup-disk/svanbot10".into())),
        ..Default::default()
    };
    let c = checks(&src);
    let micro = get(&c, "microcode");
    assert_eq!((micro.status, micro.value.as_str()), (Status::Warn, "0x27 (current 0x28)"));
    assert!(micro.advice.as_deref().unwrap().contains("intel-microcode"));
    assert!(get(&c, "cpu").value.ends_with("AVX2"));
    assert_eq!(get(&c, "thp").value, "always");
    assert_eq!(get(&c, "swap").value, "4.3 GiB of 5.8 GiB");
    assert_eq!(get(&c, "memory").value, "21.0 GiB of 31.3 GiB");
    for key in ["thp", "memory", "ssd", "archive", "fstrim"] {
        assert_eq!(get(&c, key).status, Status::Ok, "{key}");
    }
    assert_eq!(c.iter().filter(|x| x.status == Status::Warn).count(), 1);
}

#[test]
fn drift_is_flagged_with_the_command_to_fix_it() {
    let src = Sources {
        cpuinfo: haswell("0x28"),
        meminfo: meminfo(1_000_000, 6_082_556),
        thp: Some("always madvise [never]".into()),
        scaling: None,
        fstrim_enabled: Some(false),
        kernel: None,
        ssd_free: Some(5 << 30),
        archive: Some((50 << 30, false, SAME_DISK_ARCHIVE.into())),
        ..Default::default()
    };
    let c = checks(&src);
    assert_eq!(get(&c, "microcode").status, Status::Ok);
    for key in ["thp", "memory", "ssd", "archive", "fstrim"] {
        let check = get(&c, key);
        assert_eq!(check.status, Status::Warn, "{key}");
        assert!(check.advice.is_some(), "{key} has a fix");
    }
    // #725: the same-disk row stays a warning, names the dedicated folder, and the only honest fix
    // is another disk or a share — never a `/backup-disk` path this box does not have.
    let archive = get(&c, "archive");
    let advice = archive.advice.as_deref().unwrap();
    assert!(advice.contains(SAME_DISK_ARCHIVE), "the advice names the folder: {advice}");
    assert!(advice.contains("second disk") && advice.contains("network share"), "{advice}");
    assert!(advice.contains("not losing the disk"), "the disk-loss gap is stated: {advice}");
    assert!(!advice.contains("/backup-disk"), "{advice}");
    assert!(advice.contains("SVANBOT_ARCHIVE_DIR"), "{advice}");
    assert!(c.iter().all(|x| x.key != "scaling" && x.key != "kernel"), "unreadable facts are left out");
}

#[test]
fn a_filesystem_with_errors_is_flagged_and_full_swap_alone_is_not() {
    // A filesystem whose error count has climbed into the tens while the panel still says
    // "Everything as recommended", and a swap file 100% used by idle programs with no memory
    // pressure at all. Both are why this row exists.
    let src = Sources {
        meminfo: meminfo(21_000_000, 0),
        filesystems: vec![
            FsHealth { device: "sda2".into(), mount: Some("/".into()), ..Default::default() },
            FsHealth {
                device: "sdb1".into(),
                mount: Some("/backup-disk".into()),
                errors: 29,
                last_error_time: 1_000,
                last_errcode: Some(5),
                ..Default::default()
            },
        ],
        memory_pressure: Some(0.0),
        now: 1_000 + 600,
        ..Default::default()
    };
    let c = checks(&src);
    let fs = get(&c, "filesystems");
    assert_eq!(fs.status, Status::Warn);
    assert_eq!(fs.value, "/backup-disk (sdb1): 29 errors (corrupted metadata), latest 10 min ago");
    assert_eq!(get(&c, "swap").status, Status::Info, "full swap without pressure is context, not a problem");
    let pressured = checks(&Sources { memory_pressure: Some(4.0), ..src.clone() });
    assert_eq!(get(&pressured, "swap").status, Status::Warn);
    let healthy = checks(&Sources { filesystems: vec![src.filesystems[0].clone()], ..src });
    assert_eq!(get(&healthy, "filesystems").status, Status::Ok);
}

#[test]
fn the_swap_row_names_the_fleets_own_swap_and_warns_only_for_it() {
    // Swap can be 100% used by other programs and idle helpers while every SvanBot process holds
    // under 20 MB. The row has to say which of the two it is looking at.
    let base = Sources { meminfo: meminfo(21_000_000, 1_000_000), memory_pressure: Some(0.0), ..Default::default() };
    let idle = checks(&Sources { our_swap: Some(OurSwap { processes: vec![("sv10-bot (pid 42)".into(), 12 << 20)] }), ..base.clone() });
    let swap = get(&idle, "swap");
    assert_eq!(swap.status, Status::Info, "a few MB of our own pages with no pressure is context: {}", swap.value);
    assert!(swap.value.contains("ours 12 MB (sv10-bot (pid 42) 12 MB)"), "value: {}", swap.value);
    let past = checks(&Sources {
        our_swap: Some(OurSwap { processes: vec![("sv10-bot (pid 42)".into(), 700 << 20), ("sv10-bot (pid 43)".into(), 400 << 20)] }),
        ..base.clone()
    });
    let swap = get(&past, "swap");
    assert_eq!(swap.status, Status::Warn, "our own pages in swap cost the fleet: {}", swap.value);
    assert!(swap.value.contains("ours 1.1 GiB (sv10-bot (pid 42) 700 MB)"), "value: {}", swap.value);
    assert!(swap.advice.as_deref().unwrap().contains("our own pages"), "{:?}", swap.advice);
    // Our processes found with nothing of theirs in swap: a fact, and good news.
    let clear = checks(&Sources { our_swap: Some(OurSwap::default()), ..base.clone() });
    assert_eq!(get(&clear, "swap").status, Status::Info);
    assert!(get(&clear, "swap").value.contains("ours none"), "value: {}", get(&clear, "swap").value);
    // Other programs' swap with no pressure is still only context, and pressure without ours warns.
    assert!(!get(&checks(&base), "swap").value.contains("ours"), "nothing read about our processes: no claim");
    assert_eq!(get(&checks(&base), "swap").status, Status::Info);
    assert_eq!(get(&checks(&Sources { memory_pressure: Some(3.0), ..base }), "swap").status, Status::Warn);
}

#[test]
fn our_own_processes_are_recognised_by_path_or_binary_name() {
    // The paths are the deployment's, not this checkout's: a test that hard-coded a home
    // directory would assert on a string only one machine can produce, and the public tree would
    // have to rewrite it to be truthful. `/srv/svanbot10` is where the reference build installs,
    // so both trees agree on it without a substitution.
    assert!(is_ours("/srv/svanbot10/target/release/sv10-bot", "./target/release/sv10-bot"));
    assert!(is_ours("/opt/svanbot10/bin/learner", ""), "the installed bundle's path says whose it is");
    assert!(is_ours("", "sv10-bot --worker"), "a bare name is enough when the path is gone");
    assert!(!is_ours("/usr/bin/mysqld", "/usr/sbin/mysqld --daemonize"));
    assert!(!is_ours("/usr/local/bin/chrome-devtools-mcp", "node chrome-devtools-mcp"));
}

#[test]
fn another_cpu_or_an_unreadable_host_is_shown_not_judged() {
    let other = Sources {
        cpuinfo: "cpu family\t: 6\nmodel\t\t: 143\nmodel name\t: Intel Xeon\nmicrocode\t: 0x2b000603\n".into(),
        ..Sources::default()
    };
    let c = checks(&other);
    assert_eq!(get(&c, "microcode").status, Status::Info);
    let empty = checks(&Sources::default());
    assert_eq!(empty.len(), 1, "only the CPU row: {empty:?}");
    assert_eq!(empty[0].value, "unknown");
}

#[test]
fn reading_the_real_host_never_fails() {
    let dir = std::env::temp_dir();
    let src = read(&dir, &dir);
    let c = checks(&src);
    assert!(c.iter().any(|x| x.key == "cpu"));
    assert!(matches!(src.archive, Some((_, false, _)) | None), "the same directory is the same disk");
}

/// An ext4 error count alone once produced the advice "the disk is failing" and a disk replacement.
/// The drive's own SMART said otherwise: not failing, 0 bad or pending sectors, 0 cable CRC errors.
/// The 37 errors were a corrupted block bitmap (5) and one directory's bad checksum (4), with no I/O
/// error (2) — filesystem damage on a healthy disk, repaired by e2fsck, no new disk needed.
#[test]
fn filesystem_damage_on_a_healthy_disk_is_a_repair_not_a_replacement() {
    // Named once and used twice: the fixture feeds this mount point in, and the assertion below
    // looks for it coming back out. Repeating the literal would make the check depend on where the
    // host happens to keep its second disk.
    let mount = "/backup-disk";
    let fs = |first: i64, last: i64, disk: Option<DiskHealth>| Sources {
        filesystems: vec![FsHealth {
            device: "sdb1".into(),
            mount: Some(mount.into()),
            errors: 37,
            last_error_time: 1_000,
            last_errcode: Some(last),
            first_errcode: Some(first),
            disk: disk.map(|d| ("sdb".to_string(), d)),
        }],
        now: 1_600,
        ..Default::default()
    };
    let healthy = DiskHealth { failing: false, bad_sectors: 0 };
    let c = checks(&fs(5, 4, Some(healthy.clone())));
    let row = get(&c, "filesystems");
    assert_eq!(row.status, Status::Warn, "damage is still damage: writes into the broken block group fail");
    let advice = row.advice.clone().unwrap();
    assert!(advice.contains("healthy") && advice.contains("sudo e2fsck -f /dev/sdb1"), "{advice}");
    assert!(advice.contains(&format!("umount {mount}")), "the advice names the mount point: {advice}");
    assert!(!advice.contains("replace") && !advice.contains("failing"), "a healthy disk is not to be replaced: {advice}");
    assert!(row.value.contains("disk healthy"), "{}", row.value);

    // Real hardware trouble still says so: bad sectors, SMART failing, or an I/O error from the device.
    for trouble in [
        fs(5, 4, Some(DiskHealth { bad_sectors: 8, ..healthy.clone() })),
        fs(5, 4, Some(DiskHealth { failing: true, ..healthy.clone() })),
        fs(2, 4, Some(healthy.clone())),
    ] {
        let advice = get(&checks(&trouble), "filesystems").advice.clone().unwrap();
        assert!(advice.contains("replace") && advice.contains("smartctl -a /dev/sdb"), "{advice}");
    }
    // SMART unreadable: no verdict on the hardware either way, and it says how to get one.
    let unknown = get(&checks(&fs(5, 4, None)), "filesystems").advice.clone().unwrap();
    assert!(unknown.contains("smartctl") && !unknown.contains("replace the disk") && unknown.contains("e2fsck"), "{unknown}");
}

#[test]
fn udisks_smart_output_and_partition_names_are_read() {
    // The object path is `<model>_<firmware>_<serial>` and the parser must tolerate it without
    // reading anything out of it — the drive is identified by its `/dev` name, not by SMART's
    // own path. A made-up drive here, because the alternative is a real disk's serial number in a
    // published test: a serial is a unique physical device identifier, not a specification.
    let text = "  /org/freedesktop/UDisks2/drives/ExampleDrive_1TB_SN000000000:\n  org.freedesktop.UDisks2.Drive.Ata:\n    SmartEnabled:     true\n    SmartFailing:                               false\n    SmartNumBadSectors:                         0\n    SmartSupported:                             true\n";
    assert_eq!(parse_smart(text), Some(DiskHealth { failing: false, bad_sectors: 0 }));
    assert_eq!(
        parse_smart(&text.replace("SmartFailing:                               false", "SmartFailing: true")).map(|d| d.failing),
        Some(true)
    );
    assert_eq!(parse_smart("SmartSupported: false\n"), None, "no SMART is no verdict");
    assert_eq!(parse_smart(""), None);
    assert_eq!(parent_disk("sdb1"), "sdb");
    assert_eq!(parent_disk("nvme0n1p2"), "nvme0n1");
    assert_eq!(parent_disk("mmcblk0p1"), "mmcblk0");
    assert_eq!(parent_disk("sdb"), "sdb");
}

/// A package can sit at 90 °C with hours of thermal throttling already behind it while the host check
/// says nothing about heat, because nothing was reading the sensors. It says it now, and stays quiet
/// on a cool machine.
#[test]
fn a_cpu_at_its_thermal_limit_is_named_and_a_cool_one_is_not() {
    let hot = Sources { heat: Some(Heat { max_c: 90.0, throttled_ms: 25_289_300, uptime_s: 996_000 }), ..Default::default() };
    let hot_checks = checks(&hot);
    let row = get(&hot_checks, "heat");
    assert_eq!(row.status, Status::Warn);
    assert_eq!(row.value, "90 °C now; thermal throttling 7.0 h since boot (2.5% of uptime)");
    let advice = row.advice.clone().unwrap();
    assert!(advice.contains("cooler") && advice.contains("balanced"), "{advice}");
    let cool = Sources { heat: Some(Heat { max_c: 55.0, throttled_ms: 0, uptime_s: 996_000 }), ..Default::default() };
    assert_eq!(get(&checks(&cool), "heat").status, Status::Ok);
    // Hot right now but never throttled, or throttled a lot while cool now: either is worth a look.
    assert_eq!(
        get(&checks(&Sources { heat: Some(Heat { max_c: 92.0, throttled_ms: 0, uptime_s: 1_000 }), ..Default::default() }), "heat").status,
        Status::Warn
    );
    assert_eq!(
        get(
            &checks(&Sources { heat: Some(Heat { max_c: 50.0, throttled_ms: 20_000_000, uptime_s: 996_000 }), ..Default::default() }),
            "heat"
        )
        .status,
        Status::Warn
    );
    assert!(checks(&Sources::default()).iter().all(|c| c.key != "heat"), "no sensors: no row");
}
