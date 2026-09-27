//! A sampling profiler that works on this box without root: `perf` is blocked
//! (`perf_event_paranoid` 3), so `SIGPROF` from `setitimer(ITIMER_PROF)` interrupts whichever
//! thread is burning CPU every millisecond of process CPU time, and the handler records the
//! interrupted instruction pointer and, when the frame-pointer chain gives one, the caller's return
//! address. Symbols come from `addr2line -f -i -C` on a build with line tables, so inlined hot
//! loops are named with their source line.
//!
//! The handler is async-signal-safe: it only reads its `ucontext`, stores into preallocated atomics
//! and reads the stack through `process_vm_readv` on itself, which fails with `EFAULT` instead of
//! faulting when a frame pointer is garbage (code built without frame pointers, such as std).

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};

const SLOTS: usize = 1 << 21;
static SAMPLES: [AtomicU64; SLOTS] = [const { AtomicU64::new(0) }; SLOTS];
static NEXT: AtomicUsize = AtomicUsize::new(0);

extern "C" fn on_prof(_sig: libc::c_int, _info: *mut libc::siginfo_t, ctx: *mut libc::c_void) {
    // SAFETY: the kernel passes a valid `ucontext_t` to an SA_SIGINFO handler.
    let (rip, rbp) = unsafe {
        let uc = &*(ctx as *const libc::ucontext_t);
        (uc.uc_mcontext.gregs[libc::REG_RIP as usize] as u64, uc.uc_mcontext.gregs[libc::REG_RBP as usize] as u64)
    };
    let mut ret = 0u64;
    if rbp != 0 && rbp.is_multiple_of(8) {
        let local = libc::iovec { iov_base: (&mut ret as *mut u64).cast(), iov_len: 8 };
        let remote = libc::iovec { iov_base: (rbp + 8) as *mut libc::c_void, iov_len: 8 };
        // SAFETY: reads 8 bytes of our own address space into `ret`; a bad address returns an error.
        let n = unsafe { libc::process_vm_readv(libc::getpid(), &local, 1, &remote, 1, 0) };
        if n != 8 {
            ret = 0;
        }
    }
    let i = NEXT.fetch_add(2, Relaxed);
    if i + 1 < SLOTS {
        SAMPLES[i].store(rip, Relaxed);
        SAMPLES[i + 1].store(ret, Relaxed);
    }
}

/// Start sampling every `interval_us` of process CPU time.
pub fn start(interval_us: i64) {
    // SAFETY: installs a handler that is async-signal-safe (see the module docs) and arms a timer.
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = on_prof as *const () as usize;
        sa.sa_flags = libc::SA_SIGINFO | libc::SA_RESTART;
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(libc::SIGPROF, &sa, std::ptr::null_mut());
        let tv = libc::timeval { tv_sec: 0, tv_usec: interval_us };
        let it = libc::itimerval { it_interval: tv, it_value: tv };
        libc::setitimer(libc::ITIMER_PROF, &it, std::ptr::null_mut());
    }
}

/// Stop sampling.
pub fn stop() {
    // SAFETY: disarms the timer.
    unsafe {
        let zero =
            libc::itimerval { it_interval: libc::timeval { tv_sec: 0, tv_usec: 0 }, it_value: libc::timeval { tv_sec: 0, tv_usec: 0 } };
        libc::setitimer(libc::ITIMER_PROF, &zero, std::ptr::null_mut());
    }
}

/// One mapping of this process: the address range, the file offset its start corresponds to, and
/// the mapped path (empty for anonymous mappings).
struct Map {
    start: u64,
    end: u64,
    off: u64,
    path: String,
}

/// Every mapping of this process, from `/proc/self/maps`.
fn maps() -> Vec<Map> {
    let text = std::fs::read_to_string("/proc/self/maps").unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let mut f = l.splitn(6, ' ');
            let (range, _perms, off) = (f.next()?, f.next()?, f.next()?);
            let (_dev, _inode) = (f.next()?, f.next()?);
            let (a, b) = range.split_once('-')?;
            Some(Map {
                start: u64::from_str_radix(a, 16).ok()?,
                end: u64::from_str_radix(b, 16).ok()?,
                off: u64::from_str_radix(off, 16).ok()?,
                path: f.next().unwrap_or("").trim().to_string(),
            })
        })
        .collect()
}

/// Where the executable is mapped: (start, end, file offset of start) for each executable mapping.
fn exe_maps() -> Vec<(u64, u64, u64)> {
    let exe = std::fs::read_link("/proc/self/exe").unwrap_or_default();
    let exe = exe.to_string_lossy().into_owned();
    maps().iter().filter(|m| m.path.ends_with(&exe)).map(|m| (m.start, m.end, m.off)).collect()
}

/// The module an address outside the executable belongs to, and the address relative to that
/// module's file, so `addr2line` can name it from the module's own symbol table (`__libc_free`,
/// `memcpy`, …). Anonymous, stack and `[vdso]` mappings have no file to symbolize against.
fn module_of(addr: u64, maps: &[Map]) -> Option<(&str, u64)> {
    maps.iter()
        .find(|m| (m.start..m.end).contains(&addr))
        .filter(|m| m.path.starts_with('/'))
        .map(|m| (m.path.as_str(), addr - m.start + m.off))
}

/// File-relative address `addr2line` understands, for an address inside the executable. The
/// first mapping (offset 0) is the ELF base: PIE addresses are relative to it.
fn to_file(addr: u64, maps: &[(u64, u64, u64)]) -> Option<u64> {
    let base = maps.iter().find(|m| m.2 == 0)?.0;
    maps.iter().any(|(a, b, _)| (*a..*b).contains(&addr)).then(|| addr - base)
}

/// One symbolized address: the inline chain, innermost first, as `function (file:line)`.
fn symbolize_in(binary: &Path, addrs: &[u64]) -> HashMap<u64, Vec<String>> {
    let mut child = match std::process::Command::new("addr2line")
        .args(["-f", "-i", "-C", "-a", "-e"])
        .arg(binary)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };
    {
        let mut stdin = child.stdin.take().expect("piped");
        for a in addrs {
            // addr2line wants the address of the instruction, the return address minus one.
            let _ = writeln!(stdin, "{a:#x}");
        }
    }
    let out = child.wait_with_output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let mut map: HashMap<u64, Vec<String>> = HashMap::new();
    let mut current = None;
    let mut lines = out.lines().peekable();
    while let Some(l) = lines.next() {
        if let Some(hex) = l.strip_prefix("0x") {
            current = u64::from_str_radix(hex, 16).ok();
            continue;
        }
        let loc = lines.next().unwrap_or("??:0");
        let loc = loc.rsplit('/').next().unwrap_or(loc);
        if let Some(a) = current {
            map.entry(a).or_default().push(format!("{l} ({loc})"));
        }
    }
    map
}

/// Symbols for the executable itself, the common case.
fn symbolize(addrs: &[u64]) -> HashMap<u64, Vec<String>> {
    symbolize_in(&std::fs::read_link("/proc/self/exe").unwrap_or_default(), addrs)
}

/// Labels for the samples outside the executable: `<module> <function>` when the shared library's
/// symbol table names the address, else `<module>+<offset>`.
fn symbolize_modules(by_module: &HashMap<(String, u64), u64>) -> HashMap<(String, u64), String> {
    let mut per_module: HashMap<String, Vec<u64>> = HashMap::new();
    for (path, rel) in by_module.keys() {
        per_module.entry(path.clone()).or_default().push(*rel);
    }
    let mut labels = HashMap::new();
    for (path, addrs) in per_module {
        let names = if path.starts_with('/') { symbolize_in(Path::new(&path), &addrs) } else { HashMap::new() };
        let base = path.rsplit('/').next().unwrap_or(&path).to_string();
        for a in addrs {
            let label = match names.get(&a).and_then(|c| c.first()) {
                Some(f) => format!("{base}  {}", f.split(" (").next().unwrap_or("")),
                None => format!("{base}+{a:#x}"),
            };
            labels.insert((path.clone(), a), label);
        }
    }
    labels
}

/// Short function name: generic arguments and the `::h…` hash dropped, paths kept to three
/// segments. Two are not enough to tell the many `new`s apart.
fn short(f: &str) -> String {
    let name = f.split(" (").next().unwrap_or(f);
    let mut depth = 0;
    let plain: String = name
        .chars()
        .filter(|c| {
            match c {
                '<' => depth += 1,
                '>' => depth -= 1,
                _ => {}
            }
            depth == 0 && *c != '<' && *c != '>'
        })
        .collect();
    let parts: Vec<&str> = plain.split("::").filter(|p| !p.is_empty()).collect();
    let n = parts.len();
    let loc = f.rsplit(" (").next().map(|l| format!(" ({l}")).unwrap_or_default();
    format!("{}{}", parts[n.saturating_sub(3)..].join("::"), if loc.len() > 2 { loc } else { String::new() })
}

/// The report: samples by outermost function (the symbol the code lives in), by innermost inlined
/// source line, and by caller of the top functions.
pub fn report(top: usize) -> String {
    let n = NEXT.load(Relaxed).min(SLOTS) / 2;
    let all = maps();
    let maps = exe_maps();
    let mut leaf: HashMap<u64, u64> = HashMap::new();
    let mut callers: HashMap<(u64, u64), u64> = HashMap::new();
    let mut by_module: HashMap<(String, u64), u64> = HashMap::new();
    let mut outside = 0u64;
    for i in 0..n {
        let rip = SAMPLES[2 * i].load(Relaxed);
        let ret = SAMPLES[2 * i + 1].load(Relaxed);
        match to_file(rip, &maps) {
            Some(a) => {
                *leaf.entry(a).or_default() += 1;
                if let Some(r) = to_file(ret, &maps) {
                    *callers.entry((a, r.saturating_sub(1))).or_default() += 1;
                }
            }
            None => {
                outside += 1;
                let (module, rel) = module_of(rip, &all).unwrap_or(("[anonymous]", rip));
                *by_module.entry((module.to_string(), rel)).or_default() += 1;
            }
        }
    }
    let mut addrs: Vec<u64> = leaf.keys().copied().collect();
    addrs.extend(callers.keys().map(|k| k.1));
    addrs.sort_unstable();
    addrs.dedup();
    let names = symbolize(&addrs);
    let outer = |a: u64| names.get(&a).and_then(|c| c.last()).map(|s| short(s)).unwrap_or_else(|| format!("{a:#x}"));
    let inner = |a: u64| names.get(&a).and_then(|c| c.first()).map(|s| short(s)).unwrap_or_else(|| format!("{a:#x}"));
    let mut by_fn: HashMap<String, u64> = HashMap::new();
    let mut by_line: HashMap<String, u64> = HashMap::new();
    for (a, c) in &leaf {
        *by_fn.entry(outer(*a).split(" (").next().unwrap_or("").to_string()).or_default() += c;
        *by_line.entry(inner(*a)).or_default() += c;
    }
    let mut by_caller: HashMap<(String, String), u64> = HashMap::new();
    for ((a, r), c) in &callers {
        let f = outer(*a).split(" (").next().unwrap_or("").to_string();
        *by_caller.entry((f, inner(*r))).or_default() += c;
    }
    let total = n as f64;
    let mut s = format!("{n} samples ({outside} outside the executable: libc, kernel-side, vdso)\n");
    let mut table = |title: &str, rows: Vec<(String, u64)>| {
        s += &format!("\n{title}\n");
        for (k, c) in rows.into_iter().take(top) {
            s += &format!("{:6.2}%  {k}\n", c as f64 * 100.0 / total.max(1.0));
        }
    };
    let sorted = |m: HashMap<String, u64>| {
        let mut v: Vec<(String, u64)> = m.into_iter().collect();
        v.sort_by_key(|a| std::cmp::Reverse(a.1));
        v
    };
    table("by function (inlined code counted in the function it was inlined into)", sorted(by_fn.clone()));
    table("by innermost source line", sorted(by_line));
    let labels = symbolize_modules(&by_module);
    let mut outside_rows: HashMap<String, u64> = HashMap::new();
    for ((module, rel), c) in &by_module {
        let label = labels.get(&(module.clone(), *rel)).cloned().unwrap_or_else(|| format!("{module}+{rel:#x}"));
        *outside_rows.entry(label).or_default() += c;
    }
    table("outside the executable, by module", sorted(outside_rows));
    // Who calls out of the executable: the interrupted frame's return address is still in our code
    // when the instruction is in libc/libm, and that is what makes the bucket actionable.
    let mut into_exe: HashMap<u64, u64> = HashMap::new();
    for i in 0..n {
        let rip = SAMPLES[2 * i].load(Relaxed);
        let ret = SAMPLES[2 * i + 1].load(Relaxed);
        if to_file(rip, &maps).is_none()
            && let Some(r) = to_file(ret, &maps)
        {
            *into_exe.entry(r.saturating_sub(1)).or_default() += 1;
        }
    }
    let mut into_addrs: Vec<u64> = into_exe.keys().copied().collect();
    into_addrs.sort_unstable();
    let into_names = symbolize(&into_addrs);
    let mut into_rows: Vec<(String, u64)> = into_exe
        .into_iter()
        .map(|(r, c)| (into_names.get(&r).and_then(|v| v.first()).map(|s| short(s)).unwrap_or_else(|| format!("{r:#x}")), c))
        .collect();
    into_rows.sort_by_key(|a| std::cmp::Reverse(a.1));
    table("callers of the samples outside the executable (frame-pointer builds only)", into_rows);
    let hot: Vec<String> = sorted(by_fn).into_iter().take(8).map(|(k, _)| k).collect();
    let mut calls: Vec<(String, u64)> =
        by_caller.into_iter().filter(|((f, _), _)| hot.contains(f)).map(|((f, c), n)| (format!("{f}  <-  {c}"), n)).collect();
    calls.sort_by_key(|a| std::cmp::Reverse(a.1));
    table("callers of the hottest functions (frame-pointer builds only)", calls);
    s
}
