//! Allocations in one priced decision: the sample is thousands of deals, so the pricer's own
//! bookkeeping must not allocate per deal. Counted at the public seam (`price_lines_at`), with
//! the evaluator's one-time tables warmed up first.
//!
//! The counting allocator is this test binary's only `unsafe`, delegating to `System`; the
//! workspace's `unsafe_code = "deny"` is relaxed for this file alone.
#![allow(unsafe_code)]
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use svanbot_core::rules::{Legal, RaiseRange};
use svanbot_core::spot::Spot;
use svanbot_strategy::action::ActionModel;
use svanbot_strategy::pricer::{Budget, price_lines_at};

struct Counting;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static COUNTING: AtomicBool = AtomicBool::new(false);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// A turn spot with two opponents still in, both with no line.
fn spot() -> Spot {
    Spot {
        hole: "AhKh".parse().unwrap(),
        board: "QhJh2c3d".parse().unwrap(),
        pot: 400,
        legal: Legal {
            call: Some(80),
            raise: Some(RaiseRange {
                min: 160,
                max: 2_000,
            }),
        },
        opponents: 2,
        stack: 2_000,
        callable_pot: None,
        committed: 0,
        big_blind: 20,
        in_position: false,
        players_behind: None,
        lines: vec![vec![]; 2],
        seats: vec![],
    }
}

const BUDGET: Budget = Budget {
    samples: 2_000,
    seed: 5,
};

#[test]
fn a_priced_decision_allocates_a_bounded_number_of_times() {
    let at = spot();
    let model = ActionModel::baseline();
    let models = [&model, &model];
    // Warm the evaluator's one-time tables and any lazy setup outside the count.
    let _ = price_lines_at(&at, &models, BUDGET, 1.0);
    let _measuring = MEASURING.lock().unwrap();
    ALLOCATIONS.store(0, Ordering::SeqCst);
    COUNTING.store(true, Ordering::SeqCst);
    let decision = price_lines_at(&at, &models, BUDGET, 1.0);
    COUNTING.store(false, Ordering::SeqCst);
    let allocations = ALLOCATIONS.load(Ordering::SeqCst);
    assert!(decision.equity > 0.0);
    // A constant: the deal buffer, the per-decision range buffers and the ranked-pairs map. The
    // bound is the guard rail — 2,000 deals would blow through any per-deal allocation long
    // before this. What remains constant is ticketed as its own follow-up.
    assert!(
        allocations <= 200,
        "{allocations} allocations for one priced decision"
    );
}

/// Serialises the count: the counter is process-wide.
static MEASURING: Mutex<()> = Mutex::new(());

#[test]
fn the_counting_allocator_counts_an_allocation() {
    // Guards the instrument: one boxed value during the count reads as one allocation.
    let _measuring = MEASURING.lock().unwrap();
    ALLOCATIONS.store(0, Ordering::SeqCst);
    COUNTING.store(true, Ordering::SeqCst);
    let boxed = Box::new(7u64);
    COUNTING.store(false, Ordering::SeqCst);
    assert_eq!(*boxed, 7);
    assert_eq!(ALLOCATIONS.load(Ordering::SeqCst), 1);
}
