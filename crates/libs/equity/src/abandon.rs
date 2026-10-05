//! Cooperative abandonment of one decision's equity work (#750). A `spawn_blocking` search cannot be
//! cancelled: when the decision's timeout fires, the work already running keeps every core busy for
//! the rest of its search. The job installs a flag on its own thread; the timeout arm sets it; the
//! parallel chunk loops read it where they start and skip the chunks not yet begun, so the search runs
//! out in a moment and its partial result is thrown away (a short deal set or a refused equity reads
//! as "no answer", which is what the timed-out decision already is). Unset, nothing changes: the flag
//! is only ever read.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

thread_local! {
    static CURRENT: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
}

/// The flag installed on this thread, taken at the start of a parallel loop and cloned into its chunks.
pub fn current() -> Option<Arc<AtomicBool>> {
    CURRENT.with(|c| c.borrow().clone())
}

/// Whether the work should stop: a flag was installed and has been set.
pub fn abandoned(flag: &Option<Arc<AtomicBool>>) -> bool {
    flag.as_ref().is_some_and(|f| f.load(Ordering::Relaxed))
}

/// Keeps `flag` installed on this thread until dropped.
pub struct Installed(Option<Arc<AtomicBool>>);

/// Install `flag` for the calling thread's equity work.
pub fn install(flag: Arc<AtomicBool>) -> Installed {
    Installed(CURRENT.with(|c| c.borrow_mut().replace(flag)))
}

impl Drop for Installed {
    fn drop(&mut self) {
        CURRENT.with(|c| *c.borrow_mut() = self.0.take());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::equity::{SharedDeals, equity_vs_ranges_parallel};
    use sv10_cards::cards::Card;
    use sv10_cards::range::Range;
    use sv10_rng::SeedableRng;
    use sv10_rng::rngs::SmallRng;

    fn spot() -> ([Card; 2], Vec<Card>) {
        ([Card::parse("Ah").unwrap(), Card::parse("Kd").unwrap()], ["7s", "8s", "2c"].map(|c| Card::parse(c).unwrap()).to_vec())
    }

    #[test]
    fn an_unset_flag_changes_nothing_and_a_set_one_ends_the_work_with_no_answer() {
        let (hero, board) = spot();
        let full = Range::full();
        let run = || {
            let equity = equity_vs_ranges_parallel(hero, &board, &[&full], 4_000, 4, &mut SmallRng::seed_from_u64(9));
            let deals = SharedDeals::new_parallel(hero, &board, &[&full, &full], 4_000, 4, &mut SmallRng::seed_from_u64(9));
            (equity, deals.len())
        };
        let plain = run();
        assert!(plain.0.is_some() && plain.1 == 4_000, "{plain:?}");
        let flag = Arc::new(AtomicBool::new(false));
        let guard = install(flag.clone());
        assert_eq!(run(), plain, "an installed but unset flag is only ever read");
        flag.store(true, Ordering::Relaxed);
        let abandoned = run();
        assert_eq!(abandoned, (None, 0), "every chunk is skipped: no equity, an empty deal set");
        drop(guard);
        assert_eq!(run(), plain, "the flag is gone with its guard");
        assert!(current().is_none());
    }

    #[test]
    fn guards_nest_and_restore() {
        let (outer, inner) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(true)));
        let a = install(outer.clone());
        let b = install(inner.clone());
        assert!(abandoned(&current()));
        drop(b);
        assert!(!abandoned(&current()));
        drop(a);
        assert!(current().is_none());
    }
}
