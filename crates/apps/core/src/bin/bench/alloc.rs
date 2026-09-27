//! A counting global allocator: every allocation, its bytes and the peak heap, so a suite reports
//! what it allocates next to how fast it runs. Relaxed atomics: a count, not a synchronization.
//! Off unless `--allocs`: eight threads bumping shared counters put their cache line into the
//! profile (about 4% of the learner suite), so timed runs only read one never-written flag.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

pub struct Counting;

static ON: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
static LIVE: AtomicU64 = AtomicU64::new(0);
static PEAK: AtomicU64 = AtomicU64::new(0);

/// Start counting (before any measured work).
pub fn enable() {
    ON.store(true, Relaxed);
}

/// Whether counts are kept.
pub fn enabled() -> bool {
    ON.load(Relaxed)
}

#[inline]
fn count(bytes: usize) {
    if ON.load(Relaxed) {
        ALLOCS.fetch_add(1, Relaxed);
        BYTES.fetch_add(bytes as u64, Relaxed);
        grow(bytes);
    }
}

#[inline]
fn grow(by: usize) {
    let live = LIVE.fetch_add(by as u64, Relaxed) + by as u64;
    PEAK.fetch_max(live, Relaxed);
}

#[inline]
fn shrink(by: usize) {
    if ON.load(Relaxed) {
        // Saturating: memory allocated before counting began may be freed while counting.
        let _ = LIVE.fetch_update(Relaxed, Relaxed, |l| Some(l.saturating_sub(by as u64)));
    }
}

// SAFETY: every call forwards to `System` with the caller's arguments unchanged; the counters are
// side effects that cannot affect the returned memory.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        // SAFETY: forwarded as received.
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        // SAFETY: forwarded as received.
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        shrink(layout.size());
        // SAFETY: forwarded as received.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if ON.load(Relaxed) {
            ALLOCS.fetch_add(1, Relaxed);
            BYTES.fetch_add(new_size as u64, Relaxed);
            if new_size >= layout.size() {
                grow(new_size - layout.size());
            } else {
                shrink(layout.size() - new_size);
            }
        }
        // SAFETY: forwarded as received.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

/// Allocation counters at one moment.
#[derive(Clone, Copy)]
pub struct Snapshot {
    pub allocs: u64,
    pub bytes: u64,
}

pub fn snapshot() -> Snapshot {
    Snapshot { allocs: ALLOCS.load(Relaxed), bytes: BYTES.load(Relaxed) }
}

/// Reset the peak to the current live heap (so a suite reports its own peak).
pub fn reset_peak() {
    PEAK.store(LIVE.load(Relaxed), Relaxed);
}

pub fn peak_bytes() -> u64 {
    PEAK.load(Relaxed)
}
