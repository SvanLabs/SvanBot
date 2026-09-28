//! The decision-loss instrument: what the deep re-solve says each choice cost (0273, 0316, 0345).
//!
//! One responsibility — what a decision-loss *class* is, which window tests it, and when it files.
//! [`comparable_classes`] turns stored verdicts into classes; [`tested_classes`] picks the window each
//! class is tested on; [`decision_losses`] files the ones whose 95% lower bound clears
//! [`GAP_BB_PER_DECISION`], and [`measurements`] prints the rest, so a class the instrument cannot
//! test yet is a number on the panel rather than silence.
//!
//! The counterweight to 0269: a calibration residual says the model is mispriced, which costs nothing
//! when the action was right anyway. `gap_bb` says what the choice cost.
//!
//! 0355 (lesson 39): every row also names the *population* it is drawn from — the class's whole
//! decision count for the window it was tested on ([`Populations`]), the filter that decides which of
//! those decisions the deep re-solve ever sees ([`crate::replay::audit_filter`]), and, for a class the
//! filter never admits, the pot distribution that explains the silence ([`PotSpread`]) under the state
//! `never queued`. A mean over the sliver of a class the filter admits must not print in the shape of
//! the class's mean.

/// The population each row names (0355): its own file, because the rows and the population they are
/// drawn from are two responsibilities and their sum is over the 500-line rule.
mod coverage;
pub use coverage::*;

use super::Finding;
use crate::replay::audit_filter;
use std::collections::BTreeMap;

/// How large a per-decision loss has to be, over enough decisions, before it is a finding. A
/// decision costs ~45 ms and a p95 of 139 ms; 0.02 bb per decision across a thousand decisions is
/// tens of bb a day, and anything below that is inside the instrument's own noise.
pub const GAP_BB_PER_DECISION: f64 = 0.02;

/// Fewest decisions in a class before its per-decision loss means anything.
pub const GAP_MIN_DECISIONS: i64 = 500;

/// The window a class is *tested* on when it can be: short enough that a leak fixed in a release
/// clears in days rather than weeks.
pub const GAP_WINDOW_DAYS: i64 = 8;

/// The longest window a class may be measured over: the store keeps `decision_audit` for
/// [`sv10_store::store::AUDIT_RESULT_DAYS`] (30 days, `prune_audits` in the hourly backup), so a
/// longer window would silently lose its oldest verdicts. It exists for the classes that can never
/// reach [`GAP_MIN_DECISIONS`] inside [`GAP_WINDOW_DAYS`]: the all-in family accrues ~44 comparable
/// verdicts a day, not the 500 an 8-day window needs (0345).
pub const DECISION_LOSS_DAYS: i64 = 30;

// The measurement window must fit inside the retention the store prunes on, checked when this crate
// is built: a longer one loses its oldest verdicts mid-read, and the failure is silent — a class with
// 1,300 verdicts would simply measure fewer the next day, and a leak would clear itself (0345). The
// short window is the tested one and the long one the fallback, so they must not be equal either.
const _: () = assert!(
    DECISION_LOSS_DAYS <= sv10_store::store::AUDIT_RESULT_DAYS,
    "DECISION_LOSS_DAYS must not outlive the store's AUDIT_RESULT_DAYS retention: verdicts the scan measures would be pruned under it"
);
const _: () = assert!(GAP_WINDOW_DAYS < DECISION_LOSS_DAYS, "the short window is the tested one, the long one the fallback");

/// A verdict that gave up at least this many big blinds is one decision, not a drift. 98% of the
/// all-in verdicts are exactly zero and the class's whole measured loss is the handful above this
/// line, so the tally is what a reader judges such a class by (0345) — which is why every class the
/// scan prints carries it.
pub const BIG_GAP_BB: f64 = 1.0;

/// The stars a 95% interval is worth, the same z the rest of the fleet's intervals use.
pub const Z95: f64 = 1.96;

/// First replay version whose records carry every input live play used (the per-opponent
/// corrections, 0316). A verdict graded on an older record compares the live choice with a model that
/// saw less than the live choice did — a bet the fold offsets priced reads as a loss against a model
/// without them — so only verdicts at this version or later are decision-loss evidence.
pub const LIVE_INPUTS_REPLAY_VERSION: u32 = 3;

/// Which deep re-solve verdicts one decision-loss class is made of: a single (street, action family)
/// spot, or the pooled all-in family.
///
/// The pool is the one grouping that is defensible (0345): every all-in is a big pot, the analyst
/// already audits *all* of them (so waiting is the only way its count grows), the four streets share
/// the action and the candidate path a fix would touch, and alone no street reaches the sample floor
/// inside any window the store keeps. Pooling anything else would destroy the actionable signal — a
/// pooled raise class is three quarters preflop min-raises by weight, and the audited classes sit at
/// pot scales two orders apart (median pot 1.5 bb preflop, 56 bb river).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ClassKey {
    /// One street's action family.
    Spot {
        /// Street name (`preflop`, `flop`, `turn`, `river`).
        street: String,
        /// Action family, unsized (`raise`, not `raise:1605`).
        action: String,
    },
    /// Every street's all-in, as one class.
    AllIn,
}

/// The action family an all-in is (the analyst records it as `all_in:3840`, sized).
pub const ALL_IN_FAMILY: &str = "all_in";

impl ClassKey {
    /// The class's stable finding-id: the pooled class is `decision-loss:all_in`, not `…:all_in:all_in`.
    pub fn id(&self) -> String {
        match self {
            ClassKey::Spot { street, action } => format!("decision-loss:{street}:{action}"),
            ClassKey::AllIn => "decision-loss:all_in".to_string(),
        }
    }

    /// The same id for the class's measurement row, which is printed whether or not the class files.
    pub fn measurement_id(&self) -> String {
        self.id().replacen("decision-loss:", "decision-measurement:", 1)
    }

    /// How to name the class to a reader.
    pub fn label(&self) -> String {
        match self {
            ClassKey::Spot { street, action } => format!("{street} {action}"),
            ClassKey::AllIn => "all_in, every street".to_string(),
        }
    }
}

/// What one class has measured, over one window.
///
/// The second moment is carried, not just the mean: 0345 files on the 95% *lower bound* of the
/// per-decision loss, and a class like the all-ins (98% of its verdicts exactly zero) has a lower
/// bound far under its mean — which is exactly the difference between a leak and a handful of
/// decisions.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClassStat {
    /// Comparable verdicts: deep re-solves graded on records that carry the live inputs.
    pub n: i64,
    /// Big blinds given up in total (each verdict's positive part).
    pub total: f64,
    /// Sum of the squared per-decision gaps, for the interval.
    pub sumsq: f64,
    /// Verdicts that gave up at least [`BIG_GAP_BB`].
    pub big: i64,
    /// How many days of verdicts this stat covers, so a reader knows which window it is.
    pub days: i64,
}

impl ClassStat {
    /// Big blinds given up per decision.
    pub fn mean(&self) -> f64 {
        if self.n > 0 { self.total / self.n as f64 } else { 0.0 }
    }

    /// Standard error of [`mean`](Self::mean) from the class's own second moment. Infinite below two
    /// verdicts, where a sample has no spread: every test against it then fails closed.
    pub fn stderr(&self) -> f64 {
        if self.n < 2 {
            return f64::INFINITY;
        }
        let n = self.n as f64;
        let var = ((self.sumsq - self.total * self.total / n) / (n - 1.0)).max(0.0);
        (var / n).sqrt()
    }

    /// The 95% interval on the per-decision loss, or `None` on a single verdict.
    pub fn interval(&self) -> Option<(f64, f64)> {
        let se = self.stderr();
        se.is_finite().then(|| (self.mean() - Z95 * se, self.mean() + Z95 * se))
    }

    /// The 95% lower bound: what a finding has to clear, so a filed claim is one nobody has to
    /// re-check against the interval.
    pub fn lower_bound(&self) -> f64 {
        self.mean() - Z95 * self.stderr()
    }
}

/// Decision-loss classes: which spot, and what its window measured.
pub type Classes = BTreeMap<ClassKey, ClassStat>;

/// Decision-loss classes, by spot, built only from verdicts graded on records that carry the live
/// inputs ([`LIVE_INPUTS_REPLAY_VERSION`]). `days` is the window the rows were fetched over, recorded
/// on each class so a finding can say which one it rests on. Returns the number of verdicts left out,
/// so the scan can say how much of the window was not evidence.
pub fn comparable_classes<'a>(rows: impl Iterator<Item = &'a (String, sv10_store::store::AuditResult)>, days: i64) -> (Classes, usize) {
    let mut classes = Classes::new();
    let mut excluded = 0;
    for (_, r) in rows {
        if !r.replay_version.is_some_and(|v| v >= LIVE_INPUTS_REPLAY_VERSION) {
            excluded += 1;
            continue;
        }
        let action = r.live_action.split(':').next().unwrap_or(&r.live_action).to_string();
        // 0345: an all-in is evidence about the all-in family, wherever it was taken. The verdict is
        // still counted under its own street as well — the scan prints per-street lines and pools
        // only for the class that has to reach the sample floor.
        let mut keys = vec![ClassKey::Spot { street: r.street.clone(), action: action.clone() }];
        if action == ALL_IN_FAMILY {
            keys.push(ClassKey::AllIn);
        }
        let gap = r.gap_bb.max(0.0);
        for key in keys {
            let c = classes.entry(key).or_insert(ClassStat { days, ..Default::default() });
            c.n += 1;
            c.total += gap;
            c.sumsq += gap * gap;
            c.big += i64::from(gap >= BIG_GAP_BB);
        }
    }
    (classes, excluded)
}

/// The window each class is tested on (0345): the short one when it already holds
/// [`GAP_MIN_DECISIONS`] comparable verdicts there, the retention-long one otherwise. One pick, used
/// for filing, for the `sampled` counts the cleared path reads and for what the scan prints, so they
/// cannot disagree about what was measured.
pub fn tested_classes(short: &Classes, long: &Classes) -> Classes {
    long.iter()
        .map(|(key, l)| {
            let stat = match short.get(key) {
                Some(s) if s.n >= GAP_MIN_DECISIONS => *s,
                _ => *l,
            };
            (key.clone(), stat)
        })
        .collect()
}

/// What a class's window measured, in the words every decision-loss row uses: the verdict count and
/// window, the total given up, the 95% interval on the per-decision mean, and the tally of decisions
/// that gave up at least [`BIG_GAP_BB`] — the number a reader acts on when the class is mostly zeros.
///
/// 0355: it ends with the population the verdicts are drawn from, so a class's mean prints as the mean
/// of the audited sub-population it is and not as the class's (lesson 39). A class with no deep
/// re-solve at all has no mean to print and is left to [`measurements`]' `never queued` row.
///
/// #321: the filter that admits the verdicts, and what it takes to file, are the same sentence for
/// every class in the table — they live in [`decision_legend`] and are stated once per scan, where the
/// rows used to repeat them word for word.
pub fn class_evidence(key: &ClassKey, s: &ClassStat, cover: &Coverage) -> String {
    let interval = match s.interval() {
        Some((lo, hi)) => format!("95% {lo:.3}..{hi:.3}"),
        None => "no interval on one verdict".to_string(),
    };
    format!(
        "{} deep re-solves over {} days, {:.1} bb given up ({:.3} bb per decision, {interval}); {} gave up >= {BIG_GAP_BB} bb; {} [{}]",
        s.n,
        s.days,
        s.total,
        s.mean(),
        s.big,
        coverage_text(key, s.n, cover),
        key.label()
    )
}

/// The explanation every decision-loss row and the class table share (#321): what a class is, which
/// decisions the deep re-solve takes at all, and what a class has to measure before it files. Rendered
/// once above the table, and carried in full by the ticket a `P0` files, so the ticket is readable on
/// its own without thirty copies of this on the panel.
pub fn decision_legend(min_pot_bb: f64) -> String {
    format!(
        "A class is one street and action family; its verdicts are deep re-solves graded on records that carry the live inputs \
         (replay v{LIVE_INPUTS_REPLAY_VERSION} or later), and the deep re-solve takes {}. A class files as a loss only when it has \
         {GAP_MIN_DECISIONS} comparable verdicts and the 95% lower bound of its per-decision gap clears {GAP_BB_PER_DECISION} bb \
         per decision; under that it is a measurement, not a leak (0269). `Decisions in window` is the class's whole population, \
         counted over the same window its verdicts were tested on.",
        audit_filter(min_pot_bb)
    )
}

/// The population clause of a row that has a mean (0355): how much of the class the deep re-solve
/// actually saw, out of the class's whole population in the same window.
fn coverage_text(key: &ClassKey, n: i64, cover: &Coverage) -> String {
    match cover.decisions {
        None => "coverage unknown (the class's decision counts were unreadable)".to_string(),
        Some(0) => format!("no {} decisions recorded in the window", key.label()),
        Some(d) => format!("{n} of {d} decisions in the {} class in the window ({:.2}%)", key.label(), 100.0 * n as f64 / d as f64),
    }
}

/// The evidence of a class with comparable verdicts and **zero deep re-solves** in its window (0355):
/// the third state, which 0351 found `preflop:check` in — a class whose row used to print a near-zero
/// mean, indistinguishable from a class that was measured and found clean.
///
/// It names the state and the class's whole population, then the pot distribution that explains it:
/// how big the class's spots are, against the pot bar [`decision_legend`] states (0355, #321).
fn never_queued_evidence(key: &ClassKey, s: &ClassStat, cover: &Coverage) -> String {
    let population = match cover.decisions {
        Some(d) => format!("0 of {d} decisions in the {} class over the last {} days", key.label(), s.days),
        None => format!("the {} class's decisions over the last {} days could not be counted", key.label(), s.days),
    };
    // Every window verdict of a never-queued class is older than the live inputs by construction: a
    // comparable one would have made it a measurement. Naming that is what stops the pots below from
    // reading as evidence about the class.
    let pots = match cover.pots.n {
        0 => "it has no verdict in the window to show its pots at all".to_string(),
        n => format!(
            "its {n} window verdicts, all graded before replay v{LIVE_INPUTS_REPLAY_VERSION} and so not evidence, sat at a median pot of {:.1} bb (max {:.1} bb)",
            cover.pots.median, cover.pots.max
        ),
    };
    format!(
        "never queued: {population} carry a deep re-solve this instrument may use (replay v{LIVE_INPUTS_REPLAY_VERSION} or later). {pots} [{}]",
        key.label()
    )
}

/// The decision-loss findings: the deep re-solve's per-decision gap, by street and action family.
///
/// This is the instrument 0269 needed and did not have: a calibration residual says the model is
/// mispriced, which costs nothing when the action was right anyway. `gap_bb` says what the choice
/// cost. Grouped by the action *family* — a sized action (`raise:1605`) is one class, not one per
/// bet size.
///
/// 0345: a class files only when its window has [`GAP_MIN_DECISIONS`] comparable verdicts *and* the
/// 95% lower bound of its per-decision loss clears [`GAP_BB_PER_DECISION`]. The point estimate alone
/// no longer files: on a fat-tailed class it can clear the floor while the interval still contains
/// zero, and a `P0` nobody can act on is worse than a measurement.
pub fn decision_losses(classes: &Classes, cover: &Coverages) -> Vec<Finding> {
    let mut out: Vec<Finding> = classes
        .iter()
        .filter(|(_, s)| s.n >= GAP_MIN_DECISIONS && s.lower_bound() >= GAP_BB_PER_DECISION)
        .map(|(key, s)| {
            let per = s.mean();
            let c = cover.get(key).copied().unwrap_or_default();
            Finding::new(&key.id(), "P0", &format!("{} costs {per:.3} bb per decision", key.label()), class_evidence(key, s, &c), per, 0.0)
        })
        .collect();
    out.sort_by(|a, b| b.value.total_cmp(&a.value));
    out
}

/// Every class the scan measured that did not file, as a `P2` measurement (0269's rule: a
/// measurement is not a leak).
///
/// 0345: the classes the instrument cannot test — the all-in family above all — used to be invisible,
/// because a class under the floor produced no row at all. Each one now carries its verdict count,
/// window, total, interval and the tally of decisions over [`BIG_GAP_BB`], so "not yet measurable" is
/// a number on the panel instead of silence. A class that filed is left to its `P0`, so every class
/// appears exactly once.
///
/// 0355: two classes that used to read alike no longer do. A class with at least one deep re-solve
/// carries its coverage; a class with comparable verdicts and none is `never queued`, with the pot
/// distribution that explains why the filter never reaches it, rather than the mean of an empty
/// sample.
pub fn measurements(classes: &Classes, filed: &[Finding], cover: &Coverages) -> Vec<Finding> {
    let mut out: Vec<Finding> = classes
        .iter()
        .filter(|(key, _)| !filed.iter().any(|f| f.id == key.id()))
        .map(|(key, s)| {
            let c = cover.get(key).copied().unwrap_or_default();
            if s.n == 0 {
                let title = match c.decisions {
                    Some(d) => format!("{}: never queued — 0 of {d} decisions deep re-solved in {} days", key.label(), s.days),
                    None => format!("{}: never queued — no deep re-solve and no decision count in {} days", key.label(), s.days),
                };
                return Finding::new(&key.measurement_id(), "P2", &title, never_queued_evidence(key, s, &c), 0.0, 0.0);
            }
            Finding::new(
                &key.measurement_id(),
                "P2",
                &format!("{}: {:.3} bb per decision over {} deep re-solves", key.label(), s.mean(), s.n),
                class_evidence(key, s, &c),
                s.mean(),
                0.0,
            )
        })
        .collect();
    out.sort_by(|a, b| b.value.total_cmp(&a.value));
    out
}

/// One class as a table row (#321): the class, what its window measured, and what that made it. The
/// panel renders the table from these rather than out of the rows' evidence strings, and the state is
/// decided here, where the floor is — the panel never has to guess whether a number was decidable.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ClassRow {
    /// The class's finding id (`decision-loss:turn:call`), the signature the ticket carries.
    pub id: String,
    /// How to name the class to a reader (`turn call`, `all_in, every street`).
    pub label: String,
    /// Comparable verdicts in the window the class was tested on.
    pub n: i64,
    /// The window those verdicts cover, days.
    pub days: i64,
    /// Big blinds given up over the window.
    pub total: f64,
    /// Big blinds given up per decision.
    pub mean: f64,
    /// The 95% interval on [`mean`](Self::mean), or `None` on a single verdict.
    pub lo: Option<f64>,
    /// The upper end of that interval.
    pub hi: Option<f64>,
    /// Verdicts that gave up at least [`BIG_GAP_BB`].
    pub big: i64,
    /// The class's decisions in the same window, `None` when the count could not be read.
    pub decisions: Option<i64>,
    /// What the row is: `filed` (a `P0` loss), `measured` (enough verdicts, nothing over the floor),
    /// `thin` (under [`GAP_MIN_DECISIONS`], so not yet decidable) or `never queued` (no deep re-solve
    /// at all, 0355).
    pub state: String,
}

/// The classes the scan measured, as table rows (#321), ordered so the reader meets them in the order
/// they can be trusted: the filed losses by size, then the measured classes, then the ones under the
/// floor with the most evidence first, then the ones the deep re-solve never reached.
pub fn class_rows(classes: &Classes, filed: &[Finding], cover: &Coverages) -> Vec<ClassRow> {
    let state = |key: &ClassKey, s: &ClassStat| {
        if s.n == 0 {
            "never queued"
        } else if filed.iter().any(|f| f.id == key.id()) {
            "filed"
        } else if s.n < GAP_MIN_DECISIONS {
            "thin"
        } else {
            "measured"
        }
    };
    let mut out: Vec<ClassRow> = classes
        .iter()
        .map(|(key, s)| {
            let c = cover.get(key).copied().unwrap_or_default();
            let interval = s.interval();
            ClassRow {
                id: key.id(),
                label: key.label(),
                n: s.n,
                days: s.days,
                total: s.total,
                mean: s.mean(),
                lo: interval.map(|(lo, _)| lo),
                hi: interval.map(|(_, hi)| hi),
                big: s.big,
                decisions: c.decisions,
                state: state(key, s).to_string(),
            }
        })
        .collect();
    let tier = |s: &str| match s {
        "filed" => 0,
        "measured" => 1,
        "thin" => 2,
        _ => 3,
    };
    out.sort_by(|a, b| {
        tier(&a.state).cmp(&tier(&b.state)).then_with(|| match a.state.as_str() {
            // Under the floor there is no rate to rank by, only how much was measured: a class with
            // 499 verdicts is nearer to decidable than one with three, and ranking their means would
            // read noise as signal — the panel this table replaces did exactly that.
            "thin" | "never queued" => b.n.cmp(&a.n),
            _ => b.mean.total_cmp(&a.mean),
        })
    });
    out
}
