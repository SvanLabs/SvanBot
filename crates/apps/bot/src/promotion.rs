//! Promotion gate: sequential fresh-deal confirmation of the search survivor.
//!
//! The search stage picks its best candidate out of ~26, so its interval is optimistic (winner's
//! curse) and only selects. Promotion rests on fresh deals alone. A single
//! `learner_tables × learner_hands` confirmation (~24k hands, SE ~4 bb/100) could only pass
//! candidates worth about +8 bb/100 or more. Real remaining edges are smaller, so the learner
//! stalled. Confirmation now runs in chunks up to [`CONFIRM_CHUNKS`] with a Haybittle–Peto
//! design. It stops early for futility (which never raises the false-promotion rate) or for
//! overwhelming evidence (z ≥ [`EARLY_Z`]). Every promotion also requires the 95% lower bound to
//! clear the minimum worthwhile edge, keeping the overall one-sided error close to nominal 2.5%.

use sv10_core::sim::PairedResult;

/// Smallest edge worth promoting, big blinds per hand (+1 bb/100).
pub const MIN_EDGE_BB: f64 = 0.01;
/// Most confirmation chunks, each `CHUNK_SCALE × learner_tables` tables (72k hands, ~70 s on the
/// i7-4770K). 4 chunks (288k hands) left real +1.2..+1.7 bb/100 candidates just short of the +1
/// bar while the learner idled an hour between cycles (0149); 12 chunks cut the final interval
/// by √3 for at most ~14 minutes, and [`FUTILITY_FROM`] stops hopeless ones early.
pub const CONFIRM_CHUNKS: usize = 12;
/// From this chunk on, a candidate whose current mean could not clear the worthwhile edge even with
/// the full confirmation's standard error is dropped (projected-futility stop; it never promotes).
pub const FUTILITY_FROM: usize = 4;
/// Tables per confirmation chunk, as a multiple of `learner_tables`.
pub const CHUNK_SCALE: usize = 4;
/// z-score that promotes at an interim look (Haybittle–Peto boundary).
pub const EARLY_Z: f64 = 3.0;

/// Whether the search survivor is worth confirming. Its interval is selection-biased, so only
/// the point estimate is used.
pub fn worth_confirming(search: &PairedResult) -> bool {
    search.differing > 0 && search.mean_bb >= MIN_EDGE_BB
}

/// Why a confirmation rejected the candidate. One variant per branch of [`verdict`] that rejects,
/// so a branch that stops reporting its reason no longer compiles; [`Reason::code`] is what the
/// dashboard's funnel counts deaths by and [`Reason::message`] is what the log, the experiment card
/// and `review` print (#317 — the four used to be indistinguishable prose).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// The mean is not ahead of the champion at all.
    NotAhead,
    /// The 95% upper bound is below the worthwhile edge.
    UpperBelowBar,
    /// The mean could not clear the worthwhile edge even with the full confirmation's standard
    /// error (projected futility).
    CannotClear,
    /// The full confirmation's 95% lower bound is below the worthwhile edge.
    LowerBelowBar,
}

impl Reason {
    /// The dashboard's slug for this reason.
    pub fn code(self) -> &'static str {
        match self {
            Reason::NotAhead => "not-ahead",
            Reason::UpperBelowBar => "upper-below-bar",
            Reason::CannotClear => "cannot-clear",
            Reason::LowerBelowBar => "lower-below-bar",
        }
    }

    /// The sentence a person reads.
    pub fn message(self) -> &'static str {
        match self {
            Reason::NotAhead => "not ahead on fresh deals",
            Reason::UpperBelowBar => "upper bound below +1 bb/100",
            Reason::CannotClear => "cannot clear +1 bb/100 even over the full confirmation",
            Reason::LowerBelowBar => "lower bound below +1 bb/100 over the full confirmation",
        }
    }
}

/// Outcome of a confirmation look.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Promote the challenger.
    Promote,
    /// Stop without promoting, with the reason.
    Reject(Reason),
    /// Evaluate another chunk.
    Continue,
}

/// Decide after `chunk` of [`CONFIRM_CHUNKS`] chunks (1-based) from the confirmation deals so far.
pub fn verdict(r: &PairedResult, chunk: usize) -> Verdict {
    let z = if r.se_bb > 0.0 { r.mean_bb / r.se_bb } else { 0.0 };
    let clears_worthwhile_edge = r.lower_95() >= MIN_EDGE_BB;
    if chunk >= CONFIRM_CHUNKS {
        return if clears_worthwhile_edge { Verdict::Promote } else { Verdict::Reject(Reason::LowerBelowBar) };
    }
    if r.mean_bb <= 0.0 {
        return Verdict::Reject(Reason::NotAhead);
    }
    if r.upper_95() < MIN_EDGE_BB {
        return Verdict::Reject(Reason::UpperBelowBar);
    }
    if chunk >= FUTILITY_FROM {
        // The standard error shrinks with the square root of the hands still to come.
        let final_se = r.se_bb * (chunk as f64 / CONFIRM_CHUNKS as f64).sqrt();
        if r.mean_bb - 1.96 * final_se < MIN_EDGE_BB {
            return Verdict::Reject(Reason::CannotClear);
        }
    }
    if z >= EARLY_Z && clears_worthwhile_edge {
        return Verdict::Promote;
    }
    Verdict::Continue
}

#[cfg(test)]
mod tests {
    use super::*;

    fn res(mean_bb100: f64, se_bb100: f64) -> PairedResult {
        PairedResult { hands: 100_000, mean_bb: mean_bb100 / 100.0, se_bb: se_bb100 / 100.0, differing: 5_000 }
    }

    #[test]
    fn search_survivor_needs_only_a_worthwhile_point_estimate() {
        // Cycle 192's survivor: +3.92 (95% -1.08..+8.92) — used to be discarded unconfirmed.
        assert!(worth_confirming(&res(3.92, 2.55)));
        assert!(!worth_confirming(&res(0.5, 2.0)));
        assert!(!worth_confirming(&PairedResult { differing: 0, ..res(3.0, 1.0) }));
    }

    #[test]
    fn a_real_small_edge_continues_instead_of_failing_on_the_first_chunk() {
        // +4 bb/100 on one ~97k-hand chunk (SE ~2): lower bound just below zero -> keep going.
        assert_eq!(verdict(&res(4.0, 2.05), 1), Verdict::Continue);
        // Same edge over the full ~390k hands (SE ~1): promote.
        assert_eq!(verdict(&res(4.0, 1.02), CONFIRM_CHUNKS), Verdict::Promote);
    }

    #[test]
    fn interim_looks_stop_for_futility_or_overwhelming_evidence_only() {
        assert_eq!(verdict(&res(-0.5, 2.0), 1), Verdict::Reject(Reason::NotAhead));
        assert_eq!(verdict(&res(0.2, 0.3), 2), Verdict::Reject(Reason::UpperBelowBar));
        // Nominally significant (z = 2.5) at an interim look is not enough.
        assert_eq!(verdict(&res(5.0, 2.0), 2), Verdict::Continue);
        assert_eq!(verdict(&res(9.0, 2.0), 1), Verdict::Promote);
    }

    #[test]
    fn a_plus_two_edge_now_has_the_power_to_promote_and_hopeless_ones_stop_early() {
        // SE at chunk k for 72k-hand chunks: 0.82 bb/100 at 4 chunks (observed 95% half-width ±1.61
        // over 288k hands), shrinking with sqrt(k). A +2.4 edge ended 4 chunks at z 2.9, lower bound
        // +0.8: a final reject under the old gate; over 12 chunks its lower bound is +1.5.
        let se = |k: f64| 0.82 * (4.0f64 / k).sqrt();
        assert_eq!(verdict(&res(2.4, se(4.0)), 4), Verdict::Continue);
        assert_eq!(verdict(&res(2.4, se(12.0)), CONFIRM_CHUNKS), Verdict::Promote);
        // check_lookahead: +1.25 at 4 chunks cannot reach +1 + 1.96 x 0.95 by chunk 12: stop now.
        assert_eq!(verdict(&res(1.25, se(4.0)), 4), Verdict::Reject(Reason::CannotClear));
        // Before FUTILITY_FROM the projection does not apply.
        assert_eq!(verdict(&res(1.25, se(2.0)), 2), Verdict::Continue);
    }

    #[test]
    fn final_look_requires_the_worthwhile_95_percent_lower_bound() {
        assert_eq!(verdict(&res(1.5, 1.0), CONFIRM_CHUNKS), Verdict::Reject(Reason::LowerBelowBar));
        assert_eq!(verdict(&res(3.0, 1.0), CONFIRM_CHUNKS), Verdict::Promote);
        // Significant but below the minimum worthwhile edge.
        assert_eq!(verdict(&res(0.8, 0.3), CONFIRM_CHUNKS), Verdict::Reject(Reason::LowerBelowBar));
    }

    #[test]
    fn fresh_confirmation_outcomes_control_promotion() {
        assert!(worth_confirming(&res(4.0, 2.0)), "positive search survivor reaches fresh confirmation");
        assert_eq!(verdict(&res(-0.1, 0.2), 1), Verdict::Reject(Reason::NotAhead));
        assert_eq!(verdict(&res(1.96, 1.0), 1), Verdict::Continue, "a zero lower bound waits before the final look");
        assert_eq!(
            verdict(&res(1.96, 1.0), CONFIRM_CHUNKS),
            Verdict::Reject(Reason::LowerBelowBar),
            "a zero lower bound cannot promote at the final look"
        );
        assert_eq!(
            verdict(&res(2.5, 0.9), CONFIRM_CHUNKS),
            Verdict::Reject(Reason::LowerBelowBar),
            "a positive lower bound below the minimum worthwhile edge must reject"
        );
        assert_eq!(verdict(&res(3.0, 1.0), CONFIRM_CHUNKS), Verdict::Promote);
        assert_eq!(
            verdict(&res(1.2, 0.4), 1),
            Verdict::Continue,
            "z=3 alone cannot promote when the lower bound misses the worthwhile edge"
        );
    }

    #[test]
    fn every_reason_has_its_own_code_and_message() {
        // The dashboard counts rejections by code and the log prints the message (#317). Two
        // reasons sharing either would merge two different responses into one number.
        let all = [Reason::NotAhead, Reason::UpperBelowBar, Reason::CannotClear, Reason::LowerBelowBar];
        let codes: std::collections::BTreeSet<&str> = all.iter().map(|r| r.code()).collect();
        let messages: std::collections::BTreeSet<&str> = all.iter().map(|r| r.message()).collect();
        assert_eq!(codes.len(), all.len(), "codes: {codes:?}");
        assert_eq!(messages.len(), all.len(), "messages: {messages:?}");
    }
}
