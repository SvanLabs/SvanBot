//! Allocations on the equity hot path: a sampled deal must not own a heap allocation.
//!
//! Every priced decision samples `Budget::samples` deals (~2,000 in play), so one allocation per
//! deal is thousands per decision. This test counts allocations with a wrapping allocator around
//! the public seam ([`showdowns`] + [`equity_over`]) and refuses per-deal traffic.
//!
//! The counting allocator is this test binary's only `unsafe`: a textbook delegation to `System`,
//! so the workspace's `unsafe_code = "deny"` is relaxed for this file alone.
#![allow(unsafe_code)]
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use svanbot_core::card::CardSet;
use svanbot_sim::equity::{equity_over, showdowns};

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

/// Serialises the two tests that count: the counter is process-wide.
static MEASURING: Mutex<()> = Mutex::new(());

/// The allocations `work` makes.
fn allocations(work: impl FnOnce()) -> usize {
    let _measuring = MEASURING.lock().unwrap();
    ALLOCATIONS.store(0, Ordering::SeqCst);
    COUNTING.store(true, Ordering::SeqCst);
    work();
    COUNTING.store(false, Ordering::SeqCst);
    ALLOCATIONS.load(Ordering::SeqCst)
}

/// A range over every pair of aces, kings and queens, equally likely: 18 hole-card pairs.
fn range() -> Vec<(CardSet, f64)> {
    let mut cards = Vec::new();
    for rank in ["A", "K", "Q"] {
        for first in ["h", "d", "s", "c"] {
            for second in ["h", "d", "s", "c"] {
                if first < second {
                    cards.push((format!("{rank}{first}{rank}{second}").parse().unwrap(), 1.0));
                }
            }
        }
    }
    cards
}

#[test]
fn ten_thousand_deals_cost_only_the_outer_buffer() {
    let hero: CardSet = "AhKh".parse().unwrap();
    let board: CardSet = "QhJh2c".parse().unwrap();
    let range = range();
    let ranges: [&[(CardSet, f64)]; 2] = [&range, &range];
    // The evaluator builds its lookup tables on first use; that one-time cost is not the deal's.
    let _ = equity_over(&showdowns(hero, &ranges, board, 8, 1));
    let mut dealt_len = 0;
    let allocations = allocations(|| {
        let dealt = showdowns(hero, &ranges, board, 10_000, 7);
        dealt_len = dealt.len();
        assert!(equity_over(&dealt) > 0.0);
    });
    assert!(dealt_len > 5_000, "deals drawn: {dealt_len}");
    // The outer deal buffer, its growth, and the two weighted ranges: a deal itself is by value.
    assert!(
        allocations <= 32,
        "{allocations} allocations for {dealt_len} deals"
    );
}

#[test]
fn a_single_decision_sized_sample_costs_a_bounded_number_of_allocations() {
    let hero: CardSet = "AhKh".parse().unwrap();
    let board: CardSet = "QhJh2c3d".parse().unwrap();
    let range = range();
    let ranges: [&[(CardSet, f64)]; 3] = [&range, &range, &range];
    let _ = equity_over(&showdowns(hero, &ranges, board, 8, 1));
    let allocations = allocations(|| {
        let dealt = showdowns(hero, &ranges, board, 2_000, 11);
        assert_eq!(dealt.len(), 2_000);
        assert!(equity_over(&dealt) > 0.0);
    });
    assert!(
        allocations <= 32,
        "{allocations} allocations for 2,000 deals"
    );
}
