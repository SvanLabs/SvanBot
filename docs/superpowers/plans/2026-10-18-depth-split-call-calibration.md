# Depth-split call calibration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Self-calibration fits and reads flop and turn calls per pot depth (a `:deep` class at 500 bb pot + call), so the penalty fitted on deep pots stops taxing small turn pots.

**Architecture:** The rule lives once in `sv10-policy` (`calibration_key`, `DEPTH_SPLIT_CATEGORIES`, `DEEP_SUFFIX`, reusing `DEEP_CALL_MIN_POT_BB`). The store groups stored rows by a `DepthSplit` it is handed (no poker in the store); the bot passes the policy's constants through one const. Pricing reads the class only when `Params::calibration_depth_split` is set, which the live calibration task sets with the corrections, so pre-change replay records price exactly as decided.

**Tech Stack:** Rust 2024 workspace (rust 1.98), rusqlite 0.40 (bundled SQLite, `json_each`), serde/serde_json, the repo's `scripts/test.py`, `scripts/check.sh`, `sim paired`.

**Spec:** `docs/superpowers/specs/2026-10-18-depth-split-call-calibration-design.md`

## Global Constraints

- Work only in the worktree `/home/administrator/SvanBot/.scratch/depth-calibration` on branch `feat/depth-split-call-calibration`. Never edit `/home/administrator/SvanBot` (the fleet checkout).
- Build only into the worktree's own `target/dev`: `CARGO_TARGET_DIR=target/dev` (relative, AGENTS.md §2). Never `target/release`. Prefix heavy commands with `nice -n 10`. One heavy build or gate at a time.
- Depth threshold: `DEEP_CALL_MIN_POT_BB` (500.0), compared with `>=` against pot + call in big blinds.
- Split categories: exactly `"flop:call"` and `"turn:call"`. `river:call` and every other category stay pooled.
- Deep suffix: `":deep"` (classes `flop:call:deep`, `turn:call:deep`).
- No change to `calibration_update`'s rule, its caps (−15 bb, +3 bb), the margin bound, the per-pot ceiling, any live equity fit, or any learner gate.
- No Rust file may pass 500 lines; `crates/libs/policy/src/policy/mod.rs` is baselined at 705 and may not grow (`scripts/check-file-size.sh`).
- Every commit message: `<area>: <what it does>`, a why-body, `Closes #<issue>` paragraph (Task 0's issue number), then `Generated-by: claude-code/claude-opus-5-5` and `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Tests never touch the network, the live databases or `.env` (CONTRIBUTING §8). Only Task 4's evidence runs read the live store, through the same read path `review` uses.
- Deployment to the fleet is **not** part of this plan; it is a separate operator decision (#960).

## Review Focus

- A lineage's own params are adopted onto the live set at every decision (`client/decide.rs:193`, `live.adopt_promoted(own)`); if `adopt_promoted` dropped the flag, deep spots would read the shallow class while `ev_bias` holds split keys. Pinned in Task 1 (`promotion_adopts_knobs_but_keeps_local_budget_and_fits`).
- A replay record written before the change has no `calibration_depth_split` field; it must deserialize to false and price pooled, and a default `Params` must serialize without the field. Pinned in Task 3 (`a_record_from_before_the_split_prices_pooled`).
- A row at exactly 500 bb is deep in both SQL and Rust; a row with a NULL scale (pre-0207) stays in its stored category. Pinned in Task 2 (store test) and Task 3 (`the_store_groups_rows_into_the_class_pricing_reads`).
- A situation with `bb == 0` must not divide by zero or panic in the key derivation. Pinned in Task 1 (`a_zero_big_blind_prices_without_dividing_by_zero`).
- A worker process (`update_calibration_with(shared, false)`) must install the flag together with the split corrections, not only the head. Pinned in Task 3 (`a_deep_turn_penalty_no_longer_taxes_small_turn_pots`).

---

### Task 0: Open the tracking issue

**Files:** none.

- [ ] **Step 1: Create the issue as svanlabs[bot]**

```bash
cd /home/administrator/SvanBot/.scratch/depth-calibration
gh issue create --title "Fix: one pooled correction taxes small turn pots with a penalty fitted on deep ones" --label bug --body "$(cat <<'EOF'
Self-calibration fits one bb correction per category over every stored row. Flop and turn calls realize more than predicted in small pots and far less past 400 bb (30 days to 2026-10-18: turn calls +0.8 to +2.7 bb under 150 bb pot + call, -66 to -1,064 bb past 400 bb), so the pooled `turn:call` correction is -1.37 bb and lands on small turn pots whose own rows do not support it; `flop:call` reads 0 because the two signs cancel.

Fix: split flop and turn calls into a `:deep` class at 500 bb pot + call (`DEEP_CALL_MIN_POT_BB`), derived from the stored `scale`, with today's rule per class. River calls miss by a near-constant share of the pot and stay pooled. Design and the cut alternatives (a per-pot deep penalty refuted on held-out rows): `docs/superpowers/specs/2026-10-18-depth-split-call-calibration-design.md` on branch `feat/depth-split-call-calibration`.

Done when: the fit shows `turn:call` without the penalty and both deep classes with their own corrections, pre-change replay records still replay exact, the paired simulation and `scripts/check.sh full` pass. Deployment stays a separate operator decision.

Generated-by: claude-code/claude-opus-5-5
EOF
)"
```

Record the issue number printed (`https://github.com/SvanLabs/SvanBot/issues/<N>`); every commit below uses `Closes #<N>`.

---

### Task 1: The policy reads a call's calibration class by pot depth when the fit says so

**Files:**
- Create: `crates/libs/policy/src/policy/calibration.rs`
- Modify: `crates/libs/policy/src/policy/mod.rs` (module declarations near line 18; pricing loop at lines 292–299; remove `applied_bias` at lines 433–443 and its test at lines 456–468)
- Modify: `crates/libs/policy/src/policy/params.rs` (field after line 57; `Default` after line 164; `adopt_promoted` lines 212–239; `adopt_recorded_local` lines 241–266; tests lines 317–382)

**Interfaces:**
- Produces (public, re-exported from `sv10_policy::policy` and therefore `sv10_core::policy`):
  - `pub const DEPTH_SPLIT_CATEGORIES: [&str; 2]` = `["flop:call", "turn:call"]`
  - `pub const DEEP_SUFFIX: &str` = `":deep"`
  - `pub fn calibration_key(category: &str, scale_bb: f64) -> std::borrow::Cow<'_, str>`
  - `Params::calibration_depth_split: bool` (serde default false, skipped when false)
- Produces (crate-private): `pub(super) fn calibration_term(category: &str, sit: &Situation, params: &Params) -> Option<f64>` (chips)

- [ ] **Step 1: Add the field to `Params` so tests can name it**

In `crates/libs/policy/src/policy/params.rs`, directly after the `ev_bias_pot_cap` field (line 57), add:

```rust
    /// Whether `ev_bias` was fitted with flop and turn calls split by pot depth (2026-10-18), so
    /// pricing reads a call's class (`calibration_key`) rather than its pooled category. Live-fitted
    /// state set together with `ev_bias`; false in the learner, the golden snapshot and every record
    /// made before the split, which therefore price as they were decided.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub calibration_depth_split: bool,
```

In `impl Default for Params`, after `ev_bias_pot_cap: std::collections::HashMap::new(),` add:

```rust
            calibration_depth_split: false,
```

- [ ] **Step 2: Write the failing carry tests in `params.rs`**

In `promotion_adopts_knobs_but_keeps_local_budget_and_fits`, add `calibration_depth_split: true,` to the `live` literal (after its `ev_bias` line), and after `assert_eq!(live.ev_bias["river:call"], 1.5);` add:

```rust
        assert!(live.calibration_depth_split, "the split flag is local: a promoted set never clears it");
```

In `a_re_solve_adopts_the_recorded_budget_and_fits_but_keeps_its_own_knobs`, add `calibration_depth_split: true,` to the `recorded` literal (after its `ev_bias_pot_cap` line), and after `assert_eq!(knobs.ev_bias_pot_cap, recorded.ev_bias_pot_cap);` add:

```rust
        assert!(knobs.calibration_depth_split, "a re-solve prices the record's classes");
```

- [ ] **Step 3: Run them to verify they fail**

Run: `CARGO_TARGET_DIR=target/dev nice -n 10 python3 scripts/test.py -p sv10-policy -- adopt`
Expected: both tests FAIL on the new assertions (the flag is not carried yet). The first build in this worktree is cold and takes several minutes.

- [ ] **Step 4: Carry the flag in both live-set lists**

In `adopt_promoted`: extend the doc comment's list `` (`ev_bias`, `ev_bias_pot_cap`, `` to `` (`ev_bias`, `ev_bias_pot_cap`, `calibration_depth_split`, ``; after `let ev_bias_pot_cap = std::mem::take(&mut self.ev_bias_pot_cap);` add

```rust
        let calibration_depth_split = self.calibration_depth_split;
```

and after `self.ev_bias_pot_cap = ev_bias_pot_cap;` add

```rust
        self.calibration_depth_split = calibration_depth_split;
```

In `adopt_recorded_local`: extend its doc list the same way, and after `self.ev_bias_pot_cap = recorded.ev_bias_pot_cap.clone();` add

```rust
        self.calibration_depth_split = recorded.calibration_depth_split;
```

- [ ] **Step 5: Run them to verify they pass**

Run: `CARGO_TARGET_DIR=target/dev nice -n 10 python3 scripts/test.py -p sv10-policy -- adopt`
Expected: PASS.

- [ ] **Step 6: Create `calibration.rs` with the rule, the term, the moved `applied_bias`, and its tests**

Create `crates/libs/policy/src/policy/calibration.rs`:

```rust
//! Self-calibration's correction term at pricing time: which fitted class a candidate reads, and the
//! correction's size in chips.

use super::{DEEP_CALL_MIN_POT_BB, Params};
use std::borrow::Cow;
use sv10_engine::situation::Situation;

/// Categories whose calibration is fitted and read per pot depth (2026-10-18 design): flop and turn
/// calls realize more than predicted in small pots and far less past 400 bb, so one pooled
/// correction taxed the small pots with a penalty fitted on the deep ones. River calls miss by a
/// near-constant share of the pot at every depth and stay pooled.
pub const DEPTH_SPLIT_CATEGORIES: [&str; 2] = ["flop:call", "turn:call"];

/// Suffix of a split category's deep class (`turn:call:deep`).
pub const DEEP_SUFFIX: &str = ":deep";

/// The calibration class of a decision in `category` sized against `scale_bb` (pot + call, big
/// blinds): a split category's deep class from [`DEEP_CALL_MIN_POT_BB`], so "deep" names the same
/// pot as the deep-call fit's; the category itself otherwise. The store groups stored rows by the
/// same rule (`DepthSplit`, handed these constants by the bot) and a test there holds the two together.
pub fn calibration_key(category: &str, scale_bb: f64) -> Cow<'_, str> {
    if scale_bb >= DEEP_CALL_MIN_POT_BB && DEPTH_SPLIT_CATEGORIES.contains(&category) {
        Cow::Owned(format!("{category}{DEEP_SUFFIX}"))
    } else {
        Cow::Borrowed(category)
    }
}

/// The installed correction for a candidate in `category`, in chips, or `None` when its class has
/// none. Classes are read per pot depth only when `params.calibration_depth_split` says the
/// installed corrections were fitted that way, so a record made before the split prices as it did.
pub(super) fn calibration_term(category: &str, sit: &Situation, params: &Params) -> Option<f64> {
    let scale = sit.pot + sit.call_amount.max(0);
    let key = match params.calibration_depth_split {
        true => calibration_key(category, scale as f64 / sit.bb.max(1) as f64),
        false => Cow::Borrowed(category),
    };
    let bias = params.ev_bias.get(key.as_ref())?;
    Some(applied_bias(*bias, sit.bb, scale, params.ev_bias_pot_cap.get(key.as_ref()).copied()))
}

/// The live calibration's per-category correction in chips. Residuals scale with the pot (river
/// calls: -0.7 bb in pots up to 50 bb, -326 bb past 1,000 bb, 2026-09-24), so a penalty fitted in
/// big blinds never exceeds `pot_cap` (the category's supported per-pot residual) of `scale`
/// (pot + call, the calibration's unit); upward corrections are capped by the calibration step.
fn applied_bias(bias_bb: f64, bb: i64, scale: i64, pot_cap: Option<f64>) -> f64 {
    let chips = bias_bb * bb as f64;
    match pot_cap {
        Some(cap) if chips < 0.0 => chips.max(-cap * scale as f64),
        _ => chips,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sv10_engine::engine::Street;
    use sv10_model::model::ModelStore;
    use sv10_rng::SeedableRng;

    #[test]
    fn a_penalty_is_capped_by_its_own_per_pot_evidence() {
        // Live 2026-09-24: river:call carried -8.8 bb from big-pot residuals while calls in pots up to
        // 100 bb realized their prediction; its per-pot residual bounds the penalty in a small pot.
        assert_eq!(applied_bias(-8.8, 20, 280, Some(0.1)), -0.1 * 280.0);
        assert_eq!(applied_bias(-8.8, 20, 100_000, Some(0.1)), -8.8 * 20.0);
        // preflop:raise:over missed by more than the pot: its own cap leaves the bb penalty whole.
        assert_eq!(applied_bias(-1.91, 20, 60, Some(1.3)), -1.91 * 20.0);
        // No per-pot evidence (records before 0207): the bb correction as fitted.
        assert_eq!(applied_bias(-8.8, 20, 280, None), -8.8 * 20.0);
        // Upward corrections keep their own small cap in the calibration step.
        assert_eq!(applied_bias(3.0, 20, 30, Some(0.1)), 60.0);
    }

    #[test]
    fn only_flop_and_turn_calls_split_and_only_from_500_bb() {
        assert_eq!(calibration_key("turn:call", 499.99), "turn:call");
        assert_eq!(calibration_key("turn:call", 500.0), "turn:call:deep");
        assert_eq!(calibration_key("flop:call", 10_000.0), "flop:call:deep");
        assert_eq!(calibration_key("river:call", 10_000.0), "river:call", "river calls stay pooled");
        assert_eq!(calibration_key("turn:raise:big", 10_000.0), "turn:raise:big");
        assert_eq!(calibration_key("preflop:call", 10_000.0), "preflop:call");
    }

    /// A turn call in a pot of `pot` chips with `call` to pay at a 20-chip blind.
    fn turn_call(pot: i64, call: i64) -> Situation {
        let mut sit = sv10_engine::situation::fixtures::uncallable_overshove();
        sit.street = Street::Turn;
        (sit.bb, sit.pot, sit.call_amount) = (20, pot, call);
        sit
    }

    #[test]
    fn a_deep_call_reads_its_class_only_when_the_fit_was_split() {
        let split = Params {
            ev_bias: [("turn:call".to_string(), -1.0), ("turn:call:deep".to_string(), -15.0)].into_iter().collect(),
            calibration_depth_split: true,
            ..Params::default()
        };
        let deep = turn_call(9_000, 1_000); // 500 bb pot + call
        let small = turn_call(1_000, 200); // 60 bb
        assert_eq!(calibration_term("turn:call", &deep, &split), Some(-15.0 * 20.0));
        assert_eq!(calibration_term("turn:call", &small, &split), Some(-1.0 * 20.0));
        let pooled = Params { calibration_depth_split: false, ..split.clone() };
        assert_eq!(calibration_term("turn:call", &deep, &pooled), Some(-1.0 * 20.0), "a record without the flag prices pooled");
        assert_eq!(calibration_term("river:call", &deep, &split), None, "no class, no correction");
    }

    #[test]
    fn a_zero_big_blind_prices_without_dividing_by_zero() {
        let mut sit = turn_call(9_000, 1_000);
        sit.bb = 0;
        let p = Params { ev_bias: [("turn:call:deep".to_string(), -15.0)].into_iter().collect(), calibration_depth_split: true, ..Params::default() };
        assert!(calibration_term("turn:call", &sit, &p).is_some_and(f64::is_finite));
    }

    #[test]
    fn the_decision_prices_a_deep_turn_call_with_its_own_class() {
        let mut turn = sv10_engine::situation::fixtures::uncallable_overshove();
        turn.street = Street::Turn;
        turn.bb = 10;
        // After the uncallable chips go back, pot + call is about 922 bb: the deep class.
        let adj = turn.without_uncallable();
        assert!((adj.pot + adj.call_amount) as f64 / adj.bb as f64 >= DEEP_CALL_MIN_POT_BB);
        let call_bias = |flag: bool| {
            let params = Params {
                ev_bias: [("turn:call".to_string(), -1.0), ("turn:call:deep".to_string(), -15.0)].into_iter().collect(),
                calibration_depth_split: flag,
                ..Params::default()
            };
            let mut rng = sv10_rng::rngs::SmallRng::seed_from_u64(7);
            let d = crate::policy::decide_with(&turn, &ModelStore::default(), &params, None, &mut rng);
            d.candidates.iter().find(|c| c.action == "call").expect("a call is offered").bias
        };
        assert_eq!(call_bias(true), -15.0 * 10.0, "the deep class");
        assert_eq!(call_bias(false), -1.0 * 10.0, "a record without the split prices as before");
    }
}
```

- [ ] **Step 7: Wire it into `mod.rs` (net shrink)**

In `crates/libs/policy/src/policy/mod.rs`:

1. Replace

```rust
mod callers;
mod measure;
```

with

```rust
mod calibration;
mod callers;
mod measure;
```

and after `pub use params::Params;` add

```rust
pub use calibration::{DEEP_SUFFIX, DEPTH_SPLIT_CATEGORIES, calibration_key};
```

2. Replace the pricing loop body

```rust
        c.category = spot_category(sit.street, &c.action, c.amount, sit.pot, cur_bet, hero.bet);
        if let Some(b) = c.category.as_ref().and_then(|k| params.ev_bias.get(k)) {
            let cap = c.category.as_ref().and_then(|k| params.ev_bias_pot_cap.get(k)).copied();
            c.bias = applied_bias(*b, sit.bb, sit.pot + sit.call_amount.max(0), cap);
            c.ev += c.bias;
        }
```

with

```rust
        c.category = spot_category(sit.street, &c.action, c.amount, sit.pot, cur_bet, hero.bet);
        if let Some(b) = c.category.as_deref().and_then(|k| calibration::calibration_term(k, sit, params)) {
            c.bias = b;
            c.ev += b;
        }
```

3. Delete the `applied_bias` doc comment and function (from `/// The live calibration's per-category correction in chips.` through its closing `}`), and delete the test `a_penalty_is_capped_by_its_own_per_pot_evidence` from `mod tests` (it now lives in `calibration.rs`).

- [ ] **Step 8: Run the policy tests and the size check**

Run: `CARGO_TARGET_DIR=target/dev nice -n 10 python3 scripts/test.py -p sv10-policy`
Expected: PASS, including the five tests in `policy::calibration::tests`. Confirm the new names appear: `CARGO_TARGET_DIR=target/dev cargo test --profile gate -p sv10-policy -- --list 2>/dev/null | grep calibration::`.

Run: `scripts/check-file-size.sh`
Expected: exit 0 (`mod.rs` is now under 705 lines).

- [ ] **Step 9: Commit**

```bash
git add crates/libs/policy/src/policy/calibration.rs crates/libs/policy/src/policy/mod.rs crates/libs/policy/src/policy/params.rs
git commit -F - <<'EOF'
policy: a call reads its calibration class by pot depth when the fit was split

Flop and turn calls realize more than predicted in small pots and far less
past 400 bb, so one pooled correction taxes the small pots. The policy now
owns the depth rule (calibration_key: flop and turn calls from 500 bb pot +
call, the deep-call fit's threshold) and reads the class only when
Params::calibration_depth_split says the installed corrections were fitted
that way. The flag is live-fitted state, carried by adopt_promoted and
adopt_recorded_local; records made before it price exactly as decided.
Nothing sets it yet, so no decision changes in this commit.

Closes #<N>

Generated-by: claude-code/claude-opus-5-5
Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

### Task 2: The store groups calibration reads by a depth split its caller names

**Files:**
- Modify: `crates/libs/store/src/store/calibration.rs` (whole file: new type, four reads, one new test, one test call updated)
- Modify: `crates/libs/store/src/store/mod.rs:32` (re-export)
- Modify (call sites only, behaviour unchanged with `DepthSplit::NONE`): `crates/apps/bot/src/tasks/calibration.rs:94,103,109`, `crates/apps/bot/src/findings.rs:285`, `crates/apps/bot/src/derived.rs:79`, `crates/apps/bot/src/review_margins.rs:18`

**Interfaces:**
- Produces:
  - `pub struct DepthSplit { pub categories: &'static [&'static str], pub min_scale_bb: f64, pub suffix: &'static str }` with `pub const NONE: DepthSplit`
  - `Store::calibration_summary(&self, split: &DepthSplit) -> Result<Vec<CalibrationRow>>`
  - `Store::calibration_pot_summary(&self, split: &DepthSplit) -> Result<Vec<(String, i64, f64, f64)>>`
  - `Store::calibration_margin_summary(&self, split: &DepthSplit, lo: f64, hi: f64) -> Result<Vec<(String, i64, f64, f64)>>`
  - `Store::calibration_samples_since(&self, since: &str, split: &DepthSplit) -> Result<Vec<(String, f64, f64)>>`

- [ ] **Step 1: Write the failing store test**

In `crates/libs/store/src/store/calibration.rs`, inside `mod tests`, add:

```rust
    #[test]
    fn a_depth_split_groups_the_named_categories_by_scale_in_every_read() {
        let dir = std::env::temp_dir().join(format!("sv10-store-cal-split-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("svanbot10.db")).unwrap();
        for (cat, scale) in [("turn:call", 499.99), ("turn:call", 500.0), ("turn:call", 2_000.0), ("river:call", 2_000.0)] {
            store.insert_calibration("A", "h", cat, 1.0, 0.0, scale).unwrap();
        }
        // A row from before scales were recorded (0207) stays in its stored category.
        store
            .write_lock()
            .execute(
                "INSERT INTO calibration (bot, hand_id, ts, category, predicted, realized, scale)
                 VALUES ('A', 'h', '2026-01-01T00:00:00Z', 'turn:call', 1.0, 0.0, NULL)",
                [],
            )
            .unwrap();
        let split = DepthSplit { categories: &["turn:call"], min_scale_bb: 500.0, suffix: ":deep" };
        let sorted = |mut v: Vec<(String, i64)>| {
            v.sort();
            v
        };
        let summary = sorted(store.calibration_summary(&split).unwrap().into_iter().map(|r| (r.0, r.1)).collect());
        assert_eq!(summary, [("river:call".to_string(), 1), ("turn:call".into(), 2), ("turn:call:deep".into(), 2)]);
        let margin = sorted(store.calibration_margin_summary(&split, -5.0, 5.0).unwrap().into_iter().map(|r| (r.0, r.1)).collect());
        assert_eq!(margin, summary, "the margin read groups the same way");
        let pot = sorted(store.calibration_pot_summary(&split).unwrap().into_iter().map(|r| (r.0, r.1)).collect());
        assert_eq!(pot, [("river:call".to_string(), 1), ("turn:call".into(), 1), ("turn:call:deep".into(), 2)], "no scale, no pot row");
        let since: Vec<String> = store.calibration_samples_since("1970-01-01T00:00:00Z", &split).unwrap().into_iter().map(|s| s.0).collect();
        assert_eq!(since, ["turn:call", "turn:call:deep", "turn:call:deep", "river:call", "turn:call"]);
        let pooled = sorted(store.calibration_summary(&DepthSplit::NONE).unwrap().into_iter().map(|r| (r.0, r.1)).collect());
        assert_eq!(pooled, [("river:call".to_string(), 1), ("turn:call".into(), 4)], "no split: every category pooled, as before");
        let _ = std::fs::remove_dir_all(&dir);
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `CARGO_TARGET_DIR=target/dev nice -n 10 python3 scripts/test.py -p sv10-store -- depth_split`
Expected: FAIL to compile (`DepthSplit` not found; summaries take no split).

- [ ] **Step 3: Implement `DepthSplit` and the four reads**

In `crates/libs/store/src/store/calibration.rs`, change the import line `use rusqlite::params;` to

```rust
use rusqlite::{named_params, params};
```

After the `CalibrationRow` type alias, add:

```rust
/// How the calibration reads group categories by pot depth (2026-10-18): a category named in
/// `categories` is reported as `category` + `suffix` on rows whose scale (pot + call, big blinds) is
/// at least `min_scale_bb`; every other row keeps its stored category. The store applies the
/// grouping and knows no poker: which categories split, and where, is the policy's rule
/// (`calibration_key` in `sv10-policy`), handed in by the bot.
#[derive(Clone, Copy, Debug)]
pub struct DepthSplit {
    /// Stored categories that split by depth.
    pub categories: &'static [&'static str],
    /// Scale (pot + call, big blinds) from which a row is in the deep class.
    pub min_scale_bb: f64,
    /// Appended to a deep row's category.
    pub suffix: &'static str,
}

impl DepthSplit {
    /// No split: every category pooled over all depths, the grouping before 2026-10-18.
    pub const NONE: DepthSplit = DepthSplit { categories: &[], min_scale_bb: 0.0, suffix: "" };

    /// The split categories as the JSON array `json_each` reads.
    fn categories_json(&self) -> String {
        serde_json::to_string(self.categories).unwrap_or_else(|_| "[]".into())
    }
}

/// A row's calibration class in SQL; binds `:split_cats`, `:split_min` and `:split_suffix`. A row
/// without a scale (before 0207) compares as NULL and stays in its stored category.
const CLASS_SQL: &str = "CASE WHEN scale >= :split_min AND category IN (SELECT value FROM json_each(:split_cats)) \
                         THEN category || :split_suffix ELSE category END";
```

Replace `calibration_summary`:

```rust
    /// Per calibration class (`split`): (count, mean predicted, mean realized, variance of the residual).
    pub fn calibration_summary(&self, split: &DepthSplit) -> Result<Vec<CalibrationRow>> {
        let conn = self.read();
        let mut stmt = conn.prepare(&format!(
            "SELECT {CLASS_SQL} AS class, COUNT(*), AVG(predicted), AVG(realized),
                    AVG((realized - predicted) * (realized - predicted)) - AVG(realized - predicted) * AVG(realized - predicted)
             FROM calibration GROUP BY class"
        ))?;
        let cats = split.categories_json();
        let rows = stmt
            .query_map(named_params! {":split_cats": cats, ":split_min": split.min_scale_bb, ":split_suffix": split.suffix}, |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get::<_, Option<f64>>(4)?.unwrap_or(0.0)))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
```

Replace `calibration_samples_since`:

```rust
    /// Every sample recorded at or after `since` (RFC 3339) as (calibration class under `split`,
    /// predicted bb, realized bb), oldest first.
    pub fn calibration_samples_since(&self, since: &str, split: &DepthSplit) -> Result<Vec<(String, f64, f64)>> {
        let conn = self.read();
        let mut stmt =
            conn.prepare(&format!("SELECT {CLASS_SQL}, predicted, realized FROM calibration WHERE ts >= :since ORDER BY id"))?;
        let cats = split.categories_json();
        let rows = stmt
            .query_map(
                named_params! {":since": since, ":split_cats": cats, ":split_min": split.min_scale_bb, ":split_suffix": split.suffix},
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
```

Replace `calibration_margin_summary`:

```rust
    /// Per calibration class (`split`), over samples predicted in `[lo, hi)` bb (the decision
    /// margin): (count, mean residual in bb, residual variance).
    pub fn calibration_margin_summary(&self, split: &DepthSplit, lo: f64, hi: f64) -> Result<Vec<(String, i64, f64, f64)>> {
        let conn = self.read();
        let mut stmt = conn.prepare(&format!(
            "SELECT {CLASS_SQL} AS class, COUNT(*), AVG(realized - predicted),
                    AVG((realized - predicted) * (realized - predicted)) - AVG(realized - predicted) * AVG(realized - predicted)
             FROM calibration WHERE predicted >= :lo AND predicted < :hi GROUP BY class"
        ))?;
        let cats = split.categories_json();
        let rows = stmt
            .query_map(
                named_params! {":lo": lo, ":hi": hi, ":split_cats": cats, ":split_min": split.min_scale_bb, ":split_suffix": split.suffix},
                |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, Option<f64>>(2)?.unwrap_or(0.0), r.get::<_, Option<f64>>(3)?.unwrap_or(0.0))),
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
```

Replace `calibration_pot_summary`:

```rust
    /// Per calibration class (`split`), residuals as a fraction of the pot: (count, mean, variance)
    /// over rows with a known pot.
    pub fn calibration_pot_summary(&self, split: &DepthSplit) -> Result<Vec<(String, i64, f64, f64)>> {
        let conn = self.read();
        let mut stmt = conn.prepare(&format!(
            "SELECT {CLASS_SQL} AS class, COUNT(*), AVG((realized - predicted) / scale),
                    AVG(((realized - predicted) / scale) * ((realized - predicted) / scale)) - AVG((realized - predicted) / scale) * AVG((realized - predicted) / scale)
             FROM calibration WHERE scale > 0 GROUP BY class"
        ))?;
        let cats = split.categories_json();
        let rows = stmt
            .query_map(named_params! {":split_cats": cats, ":split_min": split.min_scale_bb, ":split_suffix": split.suffix}, |r| {
                Ok((r.get(0)?, r.get(1)?, r.get::<_, Option<f64>>(2)?.unwrap_or(0.0), r.get::<_, Option<f64>>(3)?.unwrap_or(0.0)))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
```

In the existing test `pot_relative_calibration_rejects_invalid_scale`, change `store.calibration_pot_summary()` to `store.calibration_pot_summary(&DepthSplit::NONE)`.

In `crates/libs/store/src/store/mod.rs`, change `pub use calibration::CalibrationRow;` to

```rust
pub use calibration::{CalibrationRow, DepthSplit};
```

- [ ] **Step 4: Keep every caller pooled (no behaviour change in this commit)**

- `crates/apps/bot/src/tasks/calibration.rs`: `shared.store.calibration_summary()` → `shared.store.calibration_summary(&sv10_store::store::DepthSplit::NONE)`; `shared.store.calibration_pot_summary()` → `shared.store.calibration_pot_summary(&sv10_store::store::DepthSplit::NONE)`; `shared.store.calibration_margin_summary(lo, hi)` → `shared.store.calibration_margin_summary(&sv10_store::store::DepthSplit::NONE, lo, hi)`.
- `crates/apps/bot/src/findings.rs:285`: `store.calibration_summary()` → `store.calibration_summary(&sv10_store::store::DepthSplit::NONE)`.
- `crates/apps/bot/src/derived.rs:79`: `.calibration_summary()?` → `.calibration_summary(&sv10_store::store::DepthSplit::NONE)?`.
- `crates/apps/bot/src/review_margins.rs:18`: `store.calibration_samples_since(&since)?` → `store.calibration_samples_since(&since, &sv10_store::store::DepthSplit::NONE)?`.

- [ ] **Step 5: Run the store and bot tests**

Run: `CARGO_TARGET_DIR=target/dev nice -n 10 python3 scripts/test.py -p sv10-store -p sv10-bot`
Expected: PASS, including `a_depth_split_groups_the_named_categories_by_scale_in_every_read`.

- [ ] **Step 6: Commit**

```bash
git add crates/libs/store/src/store/calibration.rs crates/libs/store/src/store/mod.rs crates/apps/bot/src/tasks/calibration.rs crates/apps/bot/src/findings.rs crates/apps/bot/src/derived.rs crates/apps/bot/src/review_margins.rs
git commit -F - <<'EOF'
store: calibration reads group categories by a depth split the caller names

The four calibration reads take a DepthSplit: the categories that split,
the scale (pot + call, bb) from which a row is deep, and the suffix. The
class is derived from the stored scale in SQL, so every historical row
splits at once with no schema change; a row with no scale stays pooled.
The store knows no poker: the rule is the policy's, handed in by the bot.
Every caller passes DepthSplit::NONE here, so nothing reads differently yet.

Closes #<N>

Generated-by: claude-code/claude-opus-5-5
Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

### Task 3: Self-calibration fits flop and turn calls per pot depth

**Files:**
- Modify: `crates/apps/bot/src/lib.rs` (const after line 93)
- Modify: `crates/apps/bot/src/tasks/calibration.rs` (new `fit_from_store`; `update_calibration_with`; new inline `mod tests`)
- Modify: `crates/apps/bot/src/findings.rs:167,285`
- Modify: `crates/apps/bot/src/derived.rs:79`
- Modify: `crates/apps/bot/src/review_margins.rs:18`
- Modify: `crates/apps/bot/src/review_rerun.rs:77` and its tests (lines 207–222, 253–266)
- Create: `crates/apps/bot/examples/calibration_fit.rs`
- Modify: `docs/GUIDE.md` §3.5, `docs/ARCHITECTURE.md:157-160`, `docs/CHANGELOG.md` (Unreleased)

**Interfaces:**
- Consumes: Task 1's `sv10_core::policy::{calibration_key, DEEP_SUFFIX, DEPTH_SPLIT_CATEGORIES, DEEP_CALL_MIN_POT_BB}` and `Params::calibration_depth_split`; Task 2's `sv10_store::store::DepthSplit` and the four split-taking reads.
- Produces:
  - `pub const CALIBRATION_DEPTH_SPLIT: sv10_store::store::DepthSplit` in `sv10_bot`
  - `pub fn fit_from_store(store: &Store, split: &DepthSplit) -> anyhow::Result<(HashMap<String, f64>, HashMap<String, f64>, serde_json::Map<String, serde_json::Value>)>` in `sv10_bot::tasks::calibration`
  - example binary `calibration_fit ROOT` printing per-category rows and two lines `SIM_A=<json>` / `SIM_B=<json>`

- [ ] **Step 1: Write the failing bot tests**

Append to `crates/apps/bot/src/tasks/calibration.rs`:

```rust
#[cfg(test)]
mod tests {
    use sv10_core::policy::calibration_key;

    /// The store's SQL class and the policy's key are one rule executed twice (2026-10-18 design);
    /// this holds them together at and around the threshold, so pricing always finds the class the
    /// fit wrote.
    #[test]
    fn the_store_groups_rows_into_the_class_pricing_reads() {
        let shared = crate::live::Shared::for_test("cal-split-key", &["A"]);
        let rows = [
            ("flop:call", 499.99),
            ("flop:call", 500.0),
            ("turn:call", 2_000.0),
            ("turn:call", 100.0),
            ("river:call", 2_000.0),
            ("flop:raise:big", 2_000.0),
        ];
        for (i, (cat, scale)) in rows.iter().enumerate() {
            shared.store.insert_calibration("A", &format!("h{i}"), cat, 1.0, 0.0, *scale).unwrap();
        }
        let stored: Vec<String> = shared
            .store
            .calibration_samples_since("1970-01-01T00:00:00Z", &crate::CALIBRATION_DEPTH_SPLIT)
            .unwrap()
            .into_iter()
            .map(|s| s.0)
            .collect();
        let priced: Vec<String> = rows.iter().map(|(cat, scale)| calibration_key(cat, *scale).into_owned()).collect();
        assert_eq!(stored, priced);
    }

    /// Small turn pots realize more than predicted and deep ones far less (2026-10-18). Split, the
    /// small pots lose the penalty the deep rows used to pull onto them, and every process — a
    /// worker too — installs the split flag with the corrections, so pricing reads the classes the
    /// fit wrote.
    #[test]
    fn a_deep_turn_penalty_no_longer_taxes_small_turn_pots() {
        let shared = crate::live::Shared::for_test("cal-split-fit", &["A"]);
        for i in 0..400 {
            // Small pots: +1 bb on a 60 bb call. Deep pots: -300 bb in 2,000 bb pots.
            shared.store.insert_calibration("A", &format!("s{i}"), "turn:call", 10.0, if i % 2 == 0 { 9.0 } else { 13.0 }, 60.0).unwrap();
            shared.store.insert_calibration("A", &format!("d{i}"), "turn:call", 400.0, if i % 2 == 0 { 0.0 } else { 200.0 }, 2_000.0).unwrap();
        }
        super::update_calibration_with(&shared, false);
        let params = shared.params.read();
        assert!(params.calibration_depth_split, "the worker installs the flag with the corrections");
        assert!(params.ev_bias.get("turn:call:deep").is_some_and(|b| *b < 0.0), "{:?}", params.ev_bias);
        assert!(params.ev_bias.get("turn:call").is_none_or(|b| *b >= 0.0), "small turn pots carry no penalty: {:?}", params.ev_bias);
    }

    /// A replay record written before the split carries no flag: it reads as false and keeps pricing
    /// the pooled classes, and a parameter set without the split serializes exactly as before.
    #[test]
    fn a_record_from_before_the_split_prices_pooled() {
        let old: sv10_core::policy::Params = serde_json::from_str(r#"{"ev_bias":{"turn:call":-1.0}}"#).unwrap();
        assert!(!old.calibration_depth_split);
        let json = serde_json::to_string(&sv10_core::policy::Params::default()).unwrap();
        assert!(!json.contains("calibration_depth_split"), "a default set must serialize as before the split: {json}");
    }
}
```

In `crates/apps/bot/src/review_rerun.rs` tests: in `fn recorded()`, add `calibration_depth_split: true,` after the `ev_bias_pot_cap` line; in `every_source_states_the_basis_its_parameters_have`, add `"calibration_depth_split",` as the first entry of the `differing(&current, &champ)` list (before `"deal_chunks"`).

- [ ] **Step 2: Run them to verify they fail**

Run: `CARGO_TARGET_DIR=target/dev nice -n 10 python3 scripts/test.py -p sv10-bot -- calibration::tests every_source_states`
Expected: FAIL to compile (`CALIBRATION_DEPTH_SPLIT` not found). After Step 3's const alone they would fail on the assertions (no split in the fit, flag never set).

- [ ] **Step 3: Add the bot's split const**

In `crates/apps/bot/src/lib.rs`, after `pub const CALIBRATION_KEY: &str = "calibration.v1";` add:

```rust
/// The pot-depth grouping every self-calibration read uses (2026-10-18): the policy's rule
/// (`sv10_core::policy::calibration_key`) as the store applies it.
pub const CALIBRATION_DEPTH_SPLIT: sv10_store::store::DepthSplit = sv10_store::store::DepthSplit {
    categories: &sv10_core::policy::DEPTH_SPLIT_CATEGORIES,
    min_scale_bb: sv10_core::policy::DEEP_CALL_MIN_POT_BB,
    suffix: sv10_core::policy::DEEP_SUFFIX,
};
```

- [ ] **Step 4: Fit from the split reads and install the flag**

In `crates/apps/bot/src/tasks/calibration.rs`, change `use sv10_store::store;` to

```rust
use sv10_store::store::{self, DepthSplit, Store};
```

Add above `update_calibration`:

```rust
/// What self-calibration installs from `store`'s rows with categories grouped by `split`: the bb
/// corrections, each penalty's per-pot ceiling and the dashboard table. An unreadable summary is an
/// error; unreadable pot or margin reads degrade as they always have (penalties keep their bb size,
/// no upward correction this round). The live task and `examples/calibration_fit.rs` share it.
#[allow(clippy::type_complexity)]
pub fn fit_from_store(
    store: &Store,
    split: &DepthSplit,
) -> anyhow::Result<(std::collections::HashMap<String, f64>, std::collections::HashMap<String, f64>, serde_json::Map<String, serde_json::Value>)> {
    let rows = store.calibration_summary(split)?;
    // Pot-relative residuals (0079): reported next to the bb correction, not applied, until a
    // controlled fleet split shows they improve results.
    let pot_rows = store.calibration_pot_summary(split).unwrap_or_else(|e| {
        tracing::warn!("per-pot calibration unavailable, penalties keep their bb size: {e}");
        Vec::new()
    });
    // Without margin evidence no upward correction is applied (the conservative side).
    let (lo, hi) = sv10_stats::margins::MARGIN;
    let margin_rows = store.calibration_margin_summary(split, lo, hi).unwrap_or_else(|e| {
        tracing::warn!("margin calibration unavailable, no upward corrections this round: {e}");
        Vec::new()
    });
    Ok(calibration_update(rows, pot_rows, margin_rows))
}
```

In `update_calibration_with`, replace everything from `let rows = match shared.store.calibration_summary(` through `let (bias, caps, table) = calibration_update(rows, pot_rows, margin_rows);` with:

```rust
    let (bias, caps, table) = match fit_from_store(&shared.store, &crate::CALIBRATION_DEPTH_SPLIT) {
        Ok(fit) => fit,
        Err(e) => {
            shared.log("calibration", "warn", format!("calibration summary unavailable: {e}"));
            return;
        }
    };
```

and at the end, after `params.ev_bias_pot_cap = caps;`, add:

```rust
    // The corrections were fitted per pot depth (2026-10-18); pricing reads the same classes.
    params.calibration_depth_split = true;
```

(`Shared::store` is a plain `Store`, `crates/apps/bot/src/live.rs:238`, so `&shared.store` is the `&Store` the function takes.)

- [ ] **Step 5: Point the other readers at the split**

- `crates/apps/bot/src/findings.rs:285`: `&sv10_store::store::DepthSplit::NONE` → `&crate::CALIBRATION_DEPTH_SPLIT`; and at line 167 change `format!("{n} settled decisions at every pot size")` to `format!("{n} settled decisions at every pot size its class covers")`.
- `crates/apps/bot/src/derived.rs:79`: `&sv10_store::store::DepthSplit::NONE` → `&crate::CALIBRATION_DEPTH_SPLIT`.
- `crates/apps/bot/src/review_margins.rs:18`: `&sv10_store::store::DepthSplit::NONE` → `&crate::CALIBRATION_DEPTH_SPLIT`.
- `crates/apps/bot/src/review_rerun.rs:77`: replace

```rust
    let prices = params.ev_bias == recorded.ev_bias && params.ev_bias_pot_cap == recorded.ev_bias_pot_cap && params.range == recorded.range;
```

with

```rust
    let prices = params.ev_bias == recorded.ev_bias
        && params.ev_bias_pot_cap == recorded.ev_bias_pot_cap
        && params.calibration_depth_split == recorded.calibration_depth_split
        && params.range == recorded.range;
```

- [ ] **Step 6: Run the bot tests**

Run: `CARGO_TARGET_DIR=target/dev nice -n 10 python3 scripts/test.py -p sv10-bot`
Expected: PASS, including the three `tasks::calibration::tests` and `every_source_states_the_basis_its_parameters_have`. Confirm the new names are listed (stale-binary guard): `CARGO_TARGET_DIR=target/dev cargo test --profile gate -p sv10-bot --lib -- --list 2>/dev/null | grep -c "calibration::tests"` prints `3`.

- [ ] **Step 7: Add the offline fit example**

Create `crates/apps/bot/examples/calibration_fit.rs`:

```rust
//! Offline self-calibration fit (2026-10-18 depth-split design): what the live task installs from a
//! store's rows, pooled as before the split and split by pot depth, printed per category and as the
//! two `Params` fragments `sim paired` reads (`SIM_A` pooled, `SIM_B` split). It fits and prints;
//! it stores nothing.
//!
//!   CARGO_TARGET_DIR=target/dev cargo run --profile release -p sv10-bot --example calibration_fit -- ROOT
use anyhow::{Context, Result};
use sv10_bot::tasks::calibration::fit_from_store;
use sv10_store::store::{DepthSplit, Store};

fn main() -> Result<()> {
    let root = std::path::PathBuf::from(std::env::args().nth(1).context("usage: calibration_fit ROOT")?);
    let store = Store::open(&root.join("artifacts").join("svanbot10.db"))?;
    for (arm, split, flag) in [("A", DepthSplit::NONE, false), ("B", sv10_bot::CALIBRATION_DEPTH_SPLIT, true)] {
        let (bias, caps, table) = fit_from_store(&store, &split)?;
        println!("== SIM_{arm}: {}", if flag { "split by pot depth" } else { "pooled" });
        for (key, row) in &table {
            let num = |k: &str| row[k].as_f64().unwrap_or(0.0);
            println!(
                "{key:20} n {:>7}  residual {:>9.2} ± {:>7.2}  correction {:>7.2} bb  {}",
                row["n"],
                num("residual_bb"),
                num("se_bb"),
                num("bias_bb"),
                row["bound_by"].as_str().unwrap_or("?")
            );
        }
        println!("SIM_{arm}={}", serde_json::json!({"ev_bias": bias, "ev_bias_pot_cap": caps, "calibration_depth_split": flag}));
    }
    Ok(())
}
```

Run: `CARGO_TARGET_DIR=target/dev nice -n 10 cargo clippy --profile gate -p sv10-bot --examples -- -D warnings`
Expected: no warnings.

- [ ] **Step 8: Update the documents that name what changed**

`docs/GUIDE.md` §3.5: after the sentence ending `over-optimism is what spends chips.` insert:

```markdown
Flop and turn calls are fitted per pot depth: a call in a pot of 500 bb or more (pot plus the call)
has its own class, `flop:call:deep` or `turn:call:deep`. Those calls realize far less than predicted
while small-pot calls realize slightly more, and one pooled correction used to tax the small pots with
a penalty fitted on the deep ones. River calls miss by a near-constant share of the pot at every depth
and stay one class.
```

`docs/ARCHITECTURE.md`: replace

```markdown
   (a penalty is capped at its category's supported per-pot residual, `policy::applied_bias`; an
```

with

```markdown
   (flop and turn calls read the class for their pot depth, `policy::calibration_key`, 500 bb pot + call;
   a penalty is capped at its category's supported per-pot residual, `applied_bias`; an
```

`docs/CHANGELOG.md`: directly above the first `### Fixed` heading under `## [Unreleased]` (the one after the `scripts/reap-worktrees.sh` entry), insert:

```markdown
### Changed

- **Self-calibration fits flop and turn calls per pot depth** (#<N>). A call in a pot of 500 bb or
  more has its own class (`flop:call:deep`, `turn:call:deep`), derived from the stored scale, so every
  past row splits at once. Small turn pots lose the −1.37 bb penalty deep pots had pulled onto them;
  river calls stay pooled. Replay records made before the change price exactly as decided.

```

Run: `python3 scripts/docs-check.py`
Expected: exit 0.

- [ ] **Step 9: Commit**

```bash
git add crates/apps/bot/src/lib.rs crates/apps/bot/src/tasks/calibration.rs crates/apps/bot/src/findings.rs crates/apps/bot/src/derived.rs crates/apps/bot/src/review_margins.rs crates/apps/bot/src/review_rerun.rs crates/apps/bot/examples/calibration_fit.rs docs/GUIDE.md docs/ARCHITECTURE.md docs/CHANGELOG.md
git commit -F - <<'EOF'
bot: self-calibration fits flop and turn calls per pot depth

Every calibration read now groups by the policy's depth rule, and the live
task installs Params::calibration_depth_split with the corrections it
fitted, on the head and on workers. On the stored rows this removes the
-1.37 bb penalty deep turn pots pulled onto small ones (their own rows
realize +1.9 bb over prediction) and gives deep flop and turn calls their
own correction; river calls stay pooled. examples/calibration_fit prints
the pooled and split fits and the two parameter sets the paired
simulation compares.

Closes #<N>

Generated-by: claude-code/claude-opus-5-5
Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
EOF
```

---

### Task 4: Evidence, the full gate and the pull request

**Files:** none changed unless a check fails (then fix in the owning task's files and amend nothing: add a new commit).

- [ ] **Step 1: Offline fit on the live rows**

```bash
CARGO_TARGET_DIR=target/dev nice -n 10 cargo build --profile release -p sv10-bot --example calibration_fit -p sv10-core --bin sim
target/dev/release/examples/calibration_fit /home/administrator/SvanBot | tee "$SCRATCH/calibration-fit.txt"
```

(`$SCRATCH` = `/tmp/claude-1000/-home-administrator-SvanBot/89b41092-5ae3-4518-8a57-a3db1e16fba3/scratchpad`.) This opens the live store through the same `Store::open` path `review` uses; it writes nothing.
Expected, per the spec §3.2: under SIM_A `turn:call` about −1.37 bb; under SIM_B `turn:call` 0 (absent from `ev_bias`), `turn:call:deep` and `flop:call:deep` −15 bb ("downward cap"), `river:call` unchanged. If a number differs materially, stop and report it; do not tune.

- [ ] **Step 2: Paired simulations (hard invariant)**

```bash
A=$(grep '^SIM_A=' "$SCRATCH/calibration-fit.txt" | cut -d= -f2-)
B=$(grep '^SIM_B=' "$SCRATCH/calibration-fit.txt" | cut -d= -f2-)
SIM_A="$A" SIM_B="$B" SIM_STACK_BB=100  nice -n 10 target/dev/release/sim paired 16 1500 | tee "$SCRATCH/paired-100bb.txt"
SIM_A="$A" SIM_B="$B" SIM_STACK_BB=1000 nice -n 10 target/dev/release/sim paired 16 1500 | tee "$SCRATCH/paired-1000bb.txt"
```

Expected: each prints `paired B-A over N hands: X bb/100 (95% L .. U)`. Stop condition (spec §5): an interval entirely below zero stops the change; report it rather than proceeding. Anything else is recorded as-is in the PR (a guard, not a strength claim).

- [ ] **Step 3: Golden snapshot and the full gate**

```bash
ln -s /home/administrator/SvanBot/web/node_modules web/node_modules
CARGO_TARGET_DIR=target/dev nice -n 10 scripts/check.sh full 2>&1 | tail -40
```

Expected: green. The golden snapshot must be unchanged (default `Params` has no corrections and the flag false); if the golden test fails, stop and report the diff rather than regenerating. Remove the `web/node_modules` symlink afterwards only if `git status` shows it as untracked noise (it is normally ignored).

- [ ] **Step 4: Push and open the pull request**

```bash
git push -u origin feat/depth-split-call-calibration
gh pr create --base main --title "Self-calibration fits flop and turn calls per pot depth" --body-file "$SCRATCH/pr_body.md"
```

`$SCRATCH/pr_body.md` contents: a summary (the pooled penalty, the split, river untouched), the offline fit table from Step 1 (the call rows of both arms), both paired results verbatim, the gate result, what was cut (link the spec), `Closes #<N>`, the explicit line "Deployment is a separate operator decision (#960); this PR changes no installed binary.", then

```
Generated-by: claude-code/claude-opus-5-5

🤖 Generated with [Claude Code](https://claude.com/claude-code)
```

- [ ] **Step 5: Read CI once at a decision point**

Run: `gh pr checks <PR>` after about 7 minutes (CI takes about 6.5). Expected: `web`, `gate`, `check` pass. Merge and deploy are not part of this plan; report the PR link and CI state.
