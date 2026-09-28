//! The (live -> deep) shapes inside a decision-loss class (#319): which shapes the class's
//! verdicts took, most frequent first, so a reader can tell one shape repeating from unrelated
//! hands. Split out of `decision_loss.rs` (the 500-line rule): the classes are there, what names
//! their repeats is here.

use super::{ClassKey, Classes, GAP_MIN_DECISIONS};
use std::collections::BTreeMap;

/// One (live family -> deep family) shape inside a class, with its verdict count and gap, so a
/// reader can tell one shape repeating from unrelated hands — and what the deep branch was priced
/// at when it lost (#319). Families are unsized (`raise`, not `raise:1605`), like the class.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShapeCount {
    /// Live action family.
    pub live: String,
    /// Deep action family.
    pub deep: String,
    /// Verdicts with this shape.
    pub n: i64,
    /// Big blinds those verdicts gave up in total.
    pub total_gap: f64,
}

impl ShapeCount {
    /// Big blinds given up per decision with this shape.
    pub fn mean(&self) -> f64 {
        if self.n > 0 { self.total_gap / self.n as f64 } else { 0.0 }
    }
}

/// Shapes per class, most frequent first, built beside the classes from the same verdicts.
pub type Shapes = BTreeMap<ClassKey, Vec<ShapeCount>>;

/// The action family of a sized action (`raise:1605` -> `raise`).
pub(super) fn family(action: &str) -> &str {
    action.split(':').next().unwrap_or(action)
}

/// The shapes for the window each class was tested on (#319): the same short-unless-thin pick as
/// [`tested_classes`], so a shape count never describes a window the class was not measured on.
pub fn tested_shapes(short_classes: &Classes, short: &Shapes, long: &Shapes) -> Shapes {
    long.iter()
        .map(|(key, l)| {
            let shapes = match short_classes.get(key) {
                Some(s) if s.n >= GAP_MIN_DECISIONS => short.get(key).cloned().unwrap_or_default(),
                _ => l.clone(),
            };
            (key.clone(), shapes)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::*;

    /// #319: a class's evidence names the (live -> deep) shapes behind it, most frequent first, so
    /// a reader can tell one shape repeating from unrelated hands — and what the deep branch was
    /// priced at when it lost.
    #[test]
    fn a_class_names_the_shapes_behind_its_loss() {
        let row = |live: &str, deep: &str, gap: f64| {
            (
                "2026-09-27T00:00:00Z".into(),
                sv10_store::store::AuditResult {
                    street: "river".into(),
                    live_action: live.into(),
                    deep_action: deep.into(),
                    gap_bb: gap,
                    replay_version: Some(3),
                    ..Default::default()
                },
            )
        };
        let rows = [row("check", "raise:380", 8.7), row("check", "raise:400", 9.1), row("fold", "raise:380", 1.2)];
        let (classes, shapes, excluded) = comparable_classes(rows.iter(), DECISION_LOSS_DAYS);
        assert_eq!(excluded, 0);
        let check = ClassKey::Spot { street: "river".into(), action: "check".into() };
        assert_eq!(shapes[&check].len(), 1, "one shape repeating: {:?}", shapes[&check]);
        let shape = &shapes[&check][0];
        assert_eq!((shape.live.as_str(), shape.deep.as_str(), shape.n), ("check", "raise", 2));
        assert!((shape.total_gap - 17.8).abs() < 1e-9, "{}", shape.total_gap);
        let measured = measurements(&classes, &[], &Coverages::new(), &shapes);
        let evidence = &measured.iter().find(|f| f.id == "decision-measurement:river:check").expect("a river check row").evidence;
        assert!(evidence.contains("live check -> deep raise ×2"), "{evidence}");
        assert!(evidence.contains("8.900 bb per decision"), "{evidence}");
        // The lone fold sits in its own class, named as its own shape.
        let fold = measured.iter().find(|f| f.id == "decision-measurement:river:fold").expect("a river fold row");
        assert!(fold.evidence.contains("live fold -> deep raise ×1"), "{}", fold.evidence);
    }
}
