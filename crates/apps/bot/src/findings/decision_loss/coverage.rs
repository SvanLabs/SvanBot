//! The population and coverage a decision-loss row names (0355, lesson 39): how many decisions the
//! class took, how big its spots were, and which filter decides whether the deep re-solve ever sees
//! them. Split out of `decision_loss.rs` (the 500-line rule): the rows themselves are there, the
//! population they are drawn from is here.
//!
//! [`Populations`] is the denominator, [`PotSpread`] is the explanation for a class the filter never
//! admits, and [`coverages`] joins them to the classes [`tested_classes`](super::tested_classes) chose
//! — over the same window, so a ratio and the mean beside it never describe different evidence.

use super::{ALL_IN_FAMILY, ClassKey, ClassStat, Classes, DECISION_LOSS_DAYS, GAP_WINDOW_DAYS};
use crate::replay::AUDIT_MIN_POT_BB;
use std::collections::BTreeMap;

/// How big the spots one class's window verdicts were, in big blinds (0355).
///
/// It exists for the class the deep re-solve never takes: a mean over nothing is not evidence, but the
/// pots of the verdicts such a class *does* have say why the filter never reaches it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PotSpread {
    /// Verdicts the spread is over, of every replay version, with a known positive pot.
    pub n: i64,
    /// Median pot, big blinds (the upper median of the sorted pots).
    pub median: f64,
    /// Largest pot, big blinds.
    pub max: f64,
}

/// The pot spread of every class in one window, over verdicts of *every* replay version: a class the
/// filter never admits has no comparable verdict to explain itself with, and its older verdicts are the
/// only record of how small its pots are (0355).
pub fn class_pots<'a>(rows: impl Iterator<Item = &'a (String, sv10_store::store::AuditResult)>) -> BTreeMap<ClassKey, PotSpread> {
    let mut pots: BTreeMap<ClassKey, Vec<f64>> = BTreeMap::new();
    for (_, r) in rows {
        // A verdict with no pot is not a spot size: it is left out of the spread (a NaN would sort
        // anywhere and make the median meaningless).
        if !(r.pot_bb.is_finite() && r.pot_bb > 0.0) {
            continue;
        }
        let action = r.live_action.split(':').next().unwrap_or(&r.live_action).to_string();
        // Pooled the same way the verdict itself is (0345): an all-in is evidence about the all-in
        // family wherever it was taken, and about its own street.
        let mut keys = vec![ClassKey::Spot { street: r.street.clone(), action: action.clone() }];
        if action == ALL_IN_FAMILY {
            keys.push(ClassKey::AllIn);
        }
        for key in keys {
            pots.entry(key).or_default().push(r.pot_bb);
        }
    }
    pots.into_iter()
        .map(|(key, mut v)| {
            v.sort_by(f64::total_cmp);
            let spread = PotSpread { n: v.len() as i64, median: v[v.len() / 2], max: v[v.len() - 1] };
            (key, spread)
        })
        .collect()
}

/// The class populations of both windows (0345): the short test window and the retention-long
/// fallback, each class's *whole* decision count — every decision we took in that class, whether the
/// deep re-solve solved it or not (0355).
///
/// It is the denominator of every coverage a decision-loss row names, so it is built from the store's
/// own per-(street, action) counts ([`sv10_store::store::Store::decision_counts_since`]) and pools
/// all-ins exactly as [`comparable_classes`] pools their verdicts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Populations {
    /// Counts over the short window ([`GAP_WINDOW_DAYS`]).
    pub short: BTreeMap<ClassKey, i64>,
    /// Counts over the long window ([`DECISION_LOSS_DAYS`]).
    pub long: BTreeMap<ClassKey, i64>,
}

impl Populations {
    /// Both windows' counts, from the store's rows for each: `(street, action, decisions)`, the action
    /// the unsized family live play records.
    pub fn new(long: &[(String, String, i64)], short: &[(String, String, i64)]) -> Populations {
        let classes = |rows: &[(String, String, i64)]| -> BTreeMap<ClassKey, i64> {
            let mut out: BTreeMap<ClassKey, i64> = BTreeMap::new();
            for (street, action, n) in rows {
                let mut keys = vec![ClassKey::Spot { street: street.clone(), action: action.clone() }];
                if action == ALL_IN_FAMILY {
                    keys.push(ClassKey::AllIn);
                }
                for key in keys {
                    *out.entry(key).or_insert(0) += n;
                }
            }
            out
        };
        Populations { short: classes(short), long: classes(long) }
    }

    /// The class's decisions in the window that class was tested on — the same pick [`tested_classes`]
    /// makes, so a coverage ratio and the mean beside it never describe different windows. `None` for a
    /// class neither window holds.
    pub fn tested(&self, key: &ClassKey, s: &ClassStat) -> Option<i64> {
        let window = if s.days == GAP_WINDOW_DAYS { &self.short } else { &self.long };
        window.get(key).copied()
    }

    /// The classes the scan tests, widened to every class the *long* window's population holds (0355):
    /// a class with decisions in the window and no comparable verdict gets added with nothing measured,
    /// so it can report `never queued` instead of taking no row at all — where 0351 found preflop:check,
    /// 122 verdicts and no deep re-solve after it.
    pub fn seeded(&self, tested: Classes) -> Classes {
        let mut out = tested;
        for key in self.long.keys() {
            out.entry(key.clone()).or_insert(ClassStat { days: DECISION_LOSS_DAYS, ..Default::default() });
        }
        out
    }
}

/// What a row names about the population behind it (0355, lesson 39): the class's whole decision count
/// in the window its stat covers, the pot spread of the class's window verdicts, and the floor the
/// analyst applies — the filter the deep re-solve is the sub-population of.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coverage {
    /// The class's decisions in the window, or `None` when the store's counts could not be read: a row
    /// then says the coverage is unknown rather than printing a zero nobody measured.
    pub decisions: Option<i64>,
    /// The class's window verdicts, every replay version.
    pub pots: PotSpread,
    /// The pot floor the analyst applies, big blinds ([`analyst_floor`]).
    pub min_pot_bb: f64,
}

impl Default for Coverage {
    fn default() -> Coverage {
        Coverage { decisions: None, pots: PotSpread::default(), min_pot_bb: AUDIT_MIN_POT_BB }
    }
}

/// The coverage of each class the scan is about to print, by class.
pub type Coverages = BTreeMap<ClassKey, Coverage>;

/// The coverage every class's row names: its decision population in the window it was tested on, the
/// pot spread of its window verdicts, and the analyst's floor (0355).
///
/// `populations` is `None` when the store's counts could not be read: a class then has no count, and its
/// row says so rather than pricing the mean against a denominator the scan never saw.
pub fn coverages(tested: &Classes, populations: Option<&Populations>, pots: &BTreeMap<ClassKey, PotSpread>, min_pot_bb: f64) -> Coverages {
    tested
        .iter()
        .map(|(key, s)| {
            // A read that succeeded and does not hold the class means the class took no decisions in the
            // window; a read that failed means the count is unknown. The two must not print alike.
            let decisions = populations.map(|p| p.tested(key, s).unwrap_or(0));
            let cover = Coverage { decisions, pots: pots.get(key).copied().unwrap_or_default(), min_pot_bb };
            (key.clone(), cover)
        })
        .collect()
}

/// The pot floor the analyst is applying (0355): `ANALYST_MIN_POT_BB` can override
/// [`AUDIT_MIN_POT_BB`], and a row that printed the constant while the analyst ran on something else
/// would name a filter nobody applied — the error this whole row exists to fix.
///
/// Read from the analyst's own status row, which it writes every few seconds while it runs. A status
/// that is missing, unreadable or silent about the floor falls back to the constant the analyst
/// defaults to.
pub fn analyst_floor(store: &sv10_store::store::Store) -> f64 {
    store
        .get_kv(crate::ANALYST_STATUS_KEY)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("min_pot_bb").and_then(serde_json::Value::as_f64))
        .unwrap_or(AUDIT_MIN_POT_BB)
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;

    /// One stored verdict, with the pot size the spread is over. Every version counts here: a class the
    /// filter never admits has no comparable verdict, and its older ones are its only record.
    fn verdict(street: &str, action: &str, pot_bb: f64) -> (String, sv10_store::store::AuditResult) {
        let r = sv10_store::store::AuditResult { street: street.into(), live_action: action.into(), pot_bb, ..Default::default() };
        ("2026-09-27T00:00:00Z".into(), r)
    }

    /// 0355: the denominator is the class's whole population, over the same window the mean beside it was
    /// measured on, with the all-in family pooled exactly as its verdicts are.
    #[test]
    fn the_population_counts_the_window_the_class_is_tested_on() {
        let pop = |street: &str, action: &str, n: i64| (street.to_string(), action.to_string(), n);
        let pops = Populations::new(
            &[pop("preflop", "call", 900), pop("river", "all_in", 40), pop("turn", "all_in", 10)],
            &[pop("preflop", "call", 200)],
        );
        let call = ClassKey::Spot { street: "preflop".into(), action: "call".into() };
        let short = ClassStat { n: 600, days: GAP_WINDOW_DAYS, ..Default::default() };
        let long = ClassStat { n: 40, days: DECISION_LOSS_DAYS, ..Default::default() };
        assert_eq!(pops.tested(&call, &short), Some(200), "the short window's count, for a class tested there");
        assert_eq!(pops.tested(&call, &long), Some(900), "the retention-long one for a class tested there");
        assert_eq!(pops.tested(&ClassKey::AllIn, &long), Some(50), "the pooled family sums the streets it pools");
        let nowhere = ClassKey::Spot { street: "flop".into(), action: "check".into() };
        assert_eq!(pops.tested(&nowhere, &long), None, "a class neither window holds has no count at all");
    }

    /// 0355 (0351's `preflop:check`): a class the population holds and the verdicts do not is seeded with
    /// nothing measured, so it can report `never queued` instead of taking no row at all.
    #[test]
    fn a_class_the_verdicts_do_not_reach_is_seeded_with_nothing_measured() {
        let pop = |n: i64| vec![("preflop".to_string(), "check".to_string(), n)];
        let pops = Populations::new(&pop(213), &pop(60));
        let key = ClassKey::Spot { street: "preflop".into(), action: "check".into() };
        let seeded = pops.seeded(Classes::new());
        assert_eq!((seeded[&key].n, seeded[&key].days), (0, DECISION_LOSS_DAYS), "nothing measured, over the window that keeps it");
        // A class the verdicts do reach keeps its own stat: seeding never overwrites a measurement.
        let measured = Classes::from([(key.clone(), ClassStat { n: 700, days: GAP_WINDOW_DAYS, total: 7.0, ..Default::default() })]);
        assert_eq!(pops.seeded(measured)[&key].n, 700);
    }

    /// 0355: the third state, end to end through the printers. A class with a population and no comparable
    /// verdict prints `never queued` with the population it is drawn from and the pots that explain the
    /// silence, where an empty sample used to read as a mean of zero.
    #[test]
    fn a_class_the_filter_never_reaches_prints_never_queued_and_its_pots() {
        let key = ClassKey::Spot { street: "preflop".into(), action: "check".into() };
        let rows = [
            verdict("preflop", "check", 3.0),
            verdict("preflop", "check", 2.0),
            verdict("preflop", "check", 2.0),
            verdict("preflop", "check", 1.0),
        ];
        let pots = class_pots(rows.iter());
        assert_eq!(pots[&key], PotSpread { n: 4, median: 2.0, max: 3.0 });
        let pops = Populations::new(&[("preflop".to_string(), "check".to_string(), 213)], &[]);
        let classes = pops.seeded(Classes::new());
        let cover = coverages(&classes, Some(&pops), &pots, AUDIT_MIN_POT_BB);
        let found = measurements(&classes, &[], &cover, &Shapes::new());
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].id.as_str(), found[0].severity.as_str()), ("decision-measurement:preflop:check", "P2"));
        assert!(found[0].title.contains("never queued"), "{}", found[0].title);
        for want in [
            "never queued",
            "0 of 213 decisions in the preflop check class over the last 30 days",
            &format!("its 4 window verdicts, all graded before replay v{LIVE_INPUTS_REPLAY_VERSION} and so not evidence"),
            "median pot of 2.0 bb (max 3.0 bb)",
        ] {
            assert!(found[0].evidence.contains(want), "no {want:?} in {}", found[0].evidence);
        }
        // The filter it never passes is the legend's (#321), stated once for every class, not repeated
        // into a row: the row says what the class's pots were.
        assert!(
            decision_legend(AUDIT_MIN_POT_BB)
                .contains("pot >= 50 bb, or a call of >= 12.5 bb that is at least 25% of the pot, or any all-in")
        );
        assert!(!found[0].evidence.contains("pot >= 50 bb"), "the same filter on every row is what #321 removed: {}", found[0].evidence);
    }

    /// 0355: a row that has a mean names the share of the class it saw and the filter the analyst is
    /// actually applying — the floor it reports, not the constant, when `ANALYST_MIN_POT_BB` moved it.
    #[test]
    fn a_measured_class_prints_its_coverage_and_the_filter_that_was_applied() {
        let key = ClassKey::Spot { street: "turn".into(), action: "call".into() };
        let classes = Classes::from([(key, ClassStat { n: 600, total: 18.0, sumsq: 0.54, big: 0, days: GAP_WINDOW_DAYS })]);
        let pop = |n: i64| vec![("turn".to_string(), "call".to_string(), n)];
        let pops = Populations::new(&pop(20_000), &pop(1_500));
        let cover = coverages(&classes, Some(&pops), &BTreeMap::new(), 12.5);
        let filed = decision_losses(&classes, &cover, &Shapes::new());
        assert_eq!(filed.len(), 1);
        assert!(filed[0].evidence.contains("600 of 1500 decisions in the turn call class in the window (40.00%)"), "{}", filed[0].evidence);
        assert!(
            decision_legend(12.5).contains("pot >= 12.5 bb, or a call of >= 3.125 bb that is at least 25% of the pot, or any all-in"),
            "the legend names the floor the analyst is actually applying"
        );
        assert!(measurements(&classes, &filed, &cover, &Shapes::new()).is_empty(), "the filed class keeps its one P0 row");
    }

    /// 0355: a count that could not be read is unknown, not zero — a row must not price its mean against a
    /// denominator the scan never saw.
    #[test]
    fn a_count_that_could_not_be_read_prints_as_unknown() {
        let key = ClassKey::Spot { street: "turn".into(), action: "call".into() };
        let classes = Classes::from([(key, ClassStat { n: 600, total: 18.0, sumsq: 0.54, big: 0, days: GAP_WINDOW_DAYS })]);
        let cover = coverages(&classes, None, &BTreeMap::new(), AUDIT_MIN_POT_BB);
        let filed = decision_losses(&classes, &cover, &Shapes::new());
        assert_eq!(filed.len(), 1);
        assert!(filed[0].evidence.contains("coverage unknown"), "{}", filed[0].evidence);
        assert!(!filed[0].evidence.contains("0 of "), "a zero nobody measured: {}", filed[0].evidence);
    }

    /// 0355: the floor a row prints is the one the analyst reports, so a run on `ANALYST_MIN_POT_BB` does
    /// not have its rows name the constant it overrode.
    #[test]
    fn the_printed_floor_is_the_one_the_analyst_reports() {
        let dir = std::env::temp_dir().join(format!("sv10-coverage-floor-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = sv10_store::store::Store::open(&dir.join("svanbot10.db")).unwrap();
        assert_eq!(analyst_floor(&store), AUDIT_MIN_POT_BB, "no status: the constant the analyst defaults to");
        store.put_kv(crate::ANALYST_STATUS_KEY, &serde_json::json!({"min_pot_bb": 12.5}).to_string()).unwrap();
        assert_eq!(analyst_floor(&store), 12.5);
        store.put_kv(crate::ANALYST_STATUS_KEY, &serde_json::json!({"running": true}).to_string()).unwrap();
        assert_eq!(analyst_floor(&store), AUDIT_MIN_POT_BB, "a status silent about the floor falls back");
        let _ = std::fs::remove_dir_all(&dir);
    }
    /// 0355 end to end, through the scan: every class live play takes decisions in is printed. A measured
    /// class names the share of its population its verdicts are (and the filter that chose them); a class
    /// the deep re-solve never reaches — 0351's `preflop:check`, the case that started this — says
    /// `never queued` with its population instead of a mean over an empty sample that read as clean.
    #[test]
    fn the_scan_names_each_class_population_and_the_ones_it_never_solved() {
        let dir = std::env::temp_dir().join(format!("sv10-coverage-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = sv10_store::store::Store::open(&dir.join("svanbot10.db")).unwrap();
        let conn = rusqlite::Connection::open(dir.join("svanbot10.db")).unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        for _ in 0..40 {
            stored_verdict(&tx, 1, LIVE_INPUTS_REPLAY_VERSION, "turn", "call", 0.03);
        }
        tx.commit().unwrap();
        drop(conn);
        // Live play's side of the window: the class above, and one the analyst never solved
        // (`preflop:check`, whose stored verdicts here are all pre-v3).
        let play = |street: &str, action: &str, n: usize| {
            for _ in 0..n {
                store.insert_decision("A", "h", street, action, None, Some(0.0), 0, 0, 1.0, "{}").unwrap();
            }
        };
        play("turn", "call", 200);
        play("preflop", "check", 12);

        let scan = crate::findings::scan(&store, &std::collections::HashMap::new(), 1_000.0);
        let finding = |id: &str| scan.findings.iter().find(|f| f.id == id).unwrap_or_else(|| panic!("no {id} in {:?}", scan.findings));
        let measured = finding("decision-measurement:turn:call");
        assert!(measured.evidence.contains("40 of 200 decisions in the turn call class in the window (20.00%)"), "{}", measured.evidence);
        let never = finding("decision-measurement:preflop:check");
        assert_eq!(never.severity, "P2");
        assert!(never.title.contains("never queued"), "{}", never.title);
        for want in
            ["0 of 12 decisions in the preflop check class over the last 30 days", "no verdict in the window to show its pots at all"]
        {
            assert!(never.evidence.contains(want), "no {want:?} in {}", never.evidence);
        }
        assert_eq!(scan.sampled["decision-loss:preflop:check"], 0, "and a cleared row knows it was never measured");
        // #321: the same classes are the panel's table, with the numbers the rows used to spell out and
        // the state decided here, where the floor is.
        let row =
            |label: &str| scan.classes.iter().find(|r| r.label == label).unwrap_or_else(|| panic!("no {label} in {:?}", scan.classes));
        assert_eq!((row("turn call").n, row("turn call").state.as_str(), row("turn call").decisions), (40, "thin", Some(200)));
        assert_eq!(row("preflop check").state, "never queued");
        assert!((row("turn call").mean - 0.03).abs() < 1e-9, "the rate the table prints is the one the verdicts measured");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #321: the table ranks the classes by what can be trusted of them. A class under the verdict floor
    /// is marked `thin` even when its mean is the largest on the board — the row the old panel printed
    /// above the line saying nothing could be decided.
    #[test]
    fn the_class_table_marks_what_is_not_decidable_yet() {
        let spot = |street: &str, action: &str| ClassKey::Spot { street: street.into(), action: action.into() };
        let classes = Classes::from([
            (spot("turn", "raise"), ClassStat { n: 2_000, total: 200.0, sumsq: 20.0, big: 3, days: GAP_WINDOW_DAYS }),
            (spot("turn", "call"), ClassStat { n: 600, total: 0.6, sumsq: 0.0006, big: 0, days: GAP_WINDOW_DAYS }),
            (spot("river", "call"), ClassStat { n: 40, total: 8.0, sumsq: 6.4, big: 2, days: GAP_WINDOW_DAYS }),
            (spot("preflop", "check"), ClassStat { n: 0, days: DECISION_LOSS_DAYS, ..Default::default() }),
        ]);
        let filed = decision_losses(&classes, &Coverages::new(), &Shapes::new());
        assert_eq!(filed.len(), 1, "only the class with a rate and a sample files");
        let rows = class_rows(&classes, &filed, &Coverages::new());
        assert_eq!(
            rows.iter().map(|r| r.state.as_str()).collect::<Vec<_>>(),
            ["filed", "measured", "thin", "never queued"],
            "the reader meets them in the order they can be trusted: {rows:?}"
        );
        assert_eq!(rows[2].mean, 0.2, "the thin class's rate is the largest of the four and still not a finding");
        assert_eq!(
            (rows[0].id.as_str(), rows[0].label.as_str(), rows[0].n, rows[0].big, rows[0].decisions),
            ("decision-loss:turn:raise", "turn raise", 2_000, 3, None),
            "and a count nobody could read stays unknown, not zero"
        );
        assert!(rows[0].lo.is_some() && rows[0].hi.is_some(), "the interval the table prints beside the rate");
        // One verdict has no spread: no interval is printed rather than a made-up one.
        let single = Classes::from([(spot("flop", "fold"), ClassStat { n: 1, total: 3.0, sumsq: 9.0, big: 1, days: GAP_WINDOW_DAYS })]);
        assert_eq!((class_rows(&single, &[], &Coverages::new())[0].lo, class_rows(&single, &[], &Coverages::new())[0].hi), (None, None));
    }

    /// One verdict straight into the verdict table, with the timestamp a test chooses: the queue path
    /// stamps `now`, which would put every row in every window at once.
    fn stored_verdict(tx: &rusqlite::Transaction<'_>, days_ago: i64, version: u32, street: &str, action: &str, gap: f64) {
        tx.execute(
            "INSERT INTO decision_audit (ts, bot, hand_id, street, live_action, deep_action, gap_bb, pot_bb, deep_ms, samples, replay_version)
             VALUES (?1, 'A', 'h', ?2, ?3, ?4, ?5, 0, 0, 0, ?6)",
            rusqlite::params![
                (chrono::Utc::now() - chrono::Duration::days(days_ago)).to_rfc3339(),
                street,
                action,
                action,
                gap,
                i64::from(version)
            ],
        )
        .unwrap();
    }
}
