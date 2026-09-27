//! Read-only chronological A/B evaluation of versioned opponent-response feature layouts.

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sv10_core::features::{ResponseFeatureSet, samples_from_hand_for};
use sv10_core::model::{HandSummary, ModelStore};
use sv10_core::nn::{Mlp, Sample, TrainConfig, log_loss, train};
use sv10_store::store::{HandRow, Store};

#[derive(Clone)]
struct Pair {
    incumbent: Sample,
    candidate: Sample,
    baseline: Vec<f32>,
}

fn confidence(values: &[f64]) -> (f64, f64, f64, f64) {
    let n = values.len();
    if n == 0 {
        return (f64::NAN, f64::NAN, f64::NAN, f64::NAN);
    }
    let mean = values.iter().sum::<f64>() / n as f64;
    let variance = if n > 1 { values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1) as f64 } else { 0.0 };
    let se = (variance / n as f64).sqrt();
    (mean, se, mean - 1.96 * se, mean + 1.96 * se)
}

fn split_rows(rows: &[HandRow], cutoff: &str) -> (Vec<HandRow>, Vec<HandRow>) {
    rows.iter().cloned().partition(|row| row.ended_at.as_str() < cutoff)
}

fn pairs_for_hand(row: &HandRow, models: &ModelStore, exclude: &[String]) -> Result<(HandSummary, Vec<Pair>)> {
    let mut out = Vec::new();
    let hand: HandSummary = serde_json::from_str(&row.summary).with_context(|| format!("invalid summary {}", row.hand_id))?;
    if !hand.stacks.is_empty() {
        let incumbent = samples_from_hand_for(&hand, models, exclude, ResponseFeatureSet::Incumbent37);
        let candidate = samples_from_hand_for(&hand, models, exclude, ResponseFeatureSet::PriorStreetCalls38);
        if incumbent.len() != candidate.len() {
            bail!("layout label cardinality differs for {}", row.hand_id);
        }
        for ((a, baseline), (b, _)) in incumbent.into_iter().zip(candidate) {
            if (a.label, &a.mask) != (b.label, &b.mask) {
                bail!("layout labels differ for {}", row.hand_id);
            }
            out.push(Pair { incumbent: a, candidate: b, baseline });
        }
    }
    Ok((hand, out))
}

fn chronological_pairs(rows: &[HandRow], cutoff: &str, end: Option<&str>, exclude: &[String]) -> Result<(Vec<Pair>, Vec<Pair>)> {
    let mut models = ModelStore::default();
    let mut train = Vec::new();
    let mut validation = Vec::new();
    for row in rows {
        let (hand, extracted) = pairs_for_hand(row, &models, exclude)?;
        if row.ended_at.as_str() < cutoff {
            train.extend(extracted);
        } else if end.is_none_or(|bound| row.ended_at.as_str() < bound) {
            validation.extend(extracted);
        }
        models.observe(&hand, None);
    }
    Ok((train, validation))
}

fn losses(net: &Mlp, pairs: &[Pair], candidate: bool) -> Vec<f64> {
    pairs
        .iter()
        .map(|pair| {
            let sample = if candidate { &pair.candidate } else { &pair.incumbent };
            log_loss(&net.predict(&sample.x, &sample.mask), sample.label)
        })
        .collect()
}

fn slice_report(net_a: &Mlp, net_b: &Mlp, pairs: &[Pair], keep: impl Fn(&Pair) -> bool) -> Value {
    let selected = pairs.iter().filter(|pair| keep(pair)).cloned().collect::<Vec<_>>();
    if selected.is_empty() {
        return json!({"samples": 0});
    }
    let a = losses(net_a, &selected, false);
    let b = losses(net_b, &selected, true);
    let delta = a.iter().zip(&b).map(|(left, right)| right - left).collect::<Vec<_>>();
    let (mean, se, low, high) = confidence(&delta);
    json!({"samples": selected.len(), "incumbent_loss": confidence(&a).0, "candidate_loss": confidence(&b).0,
        "paired_delta": mean, "standard_error": se, "ci95": [low, high]})
}

fn repeated_call_report(net_a: &Mlp, net_b: &Mlp, pairs: &[Pair]) -> Value {
    let selected = pairs.iter().filter(|pair| pair.candidate.x[3] == 1.0 && pair.candidate.x[37] > 0.0).collect::<Vec<_>>();
    if selected.is_empty() {
        return json!({"samples": 0});
    }
    let mut losses_a = Vec::new();
    let mut losses_b = Vec::new();
    let mut probs_a = Vec::new();
    let mut probs_b = Vec::new();
    let mut outcomes = Vec::new();
    for pair in selected {
        let pa = net_a.predict(&pair.incumbent.x, &pair.incumbent.mask)[0].clamp(1e-7, 1.0 - 1e-7) as f64;
        let pb = net_b.predict(&pair.candidate.x, &pair.candidate.mask)[0].clamp(1e-7, 1.0 - 1e-7) as f64;
        let y = f64::from(pair.incumbent.label == 0);
        losses_a.push(-(y * pa.ln() + (1.0 - y) * (1.0 - pa).ln()));
        losses_b.push(-(y * pb.ln() + (1.0 - y) * (1.0 - pb).ln()));
        probs_a.push(pa);
        probs_b.push(pb);
        outcomes.push(y);
    }
    let delta = losses_a.iter().zip(&losses_b).map(|(a, b)| b - a).collect::<Vec<_>>();
    let (mean, se, low, high) = confidence(&delta);
    let observed = confidence(&outcomes).0;
    json!({"samples": outcomes.len(), "incumbent_binary_loss": confidence(&losses_a).0,
        "candidate_binary_loss": confidence(&losses_b).0,
        "incumbent_calibration_error": (confidence(&probs_a).0-observed).abs(),
        "candidate_calibration_error": (confidence(&probs_b).0-observed).abs(),
        "paired_delta": mean, "standard_error": se, "ci95": [low, high]})
}

fn evaluate(train_pairs: &[Pair], validation: &[Pair], seed: u64, epochs: usize) -> Result<Value> {
    if validation.len() < 300 {
        bail!("need at least 300 validation samples, got {}", validation.len());
    }
    let train_a = train_pairs.iter().map(|pair| pair.incumbent.clone()).collect::<Vec<_>>();
    let train_b = train_pairs.iter().map(|pair| pair.candidate.clone()).collect::<Vec<_>>();
    let mut net_a = Mlp::new(&[37, 48, 24, 3], seed);
    let mut net_b = Mlp::new(&[38, 48, 24, 3], seed);
    let config = TrainConfig { epochs, seed, ..Default::default() };
    train(&mut net_a, &train_a, &config);
    train(&mut net_b, &train_b, &config);
    let a = losses(&net_a, validation, false);
    let b = losses(&net_b, validation, true);
    let baseline = validation.iter().map(|pair| log_loss(&pair.baseline, pair.incumbent.label)).collect::<Vec<_>>();
    let delta = a.iter().zip(&b).map(|(left, right)| right - left).collect::<Vec<_>>();
    let (mean, se, low, high) = confidence(&delta);
    let streets = ["preflop", "flop", "turn", "river"]
        .iter()
        .enumerate()
        .map(|(index, name)| ((*name).to_string(), slice_report(&net_a, &net_b, validation, |pair| pair.incumbent.x[index] == 1.0)))
        .collect::<serde_json::Map<_, _>>();
    let confidence_slices = [("low", 0.0, 0.34), ("medium", 0.34, 0.67), ("high", 0.67, 1.01)]
        .into_iter()
        .map(|(name, low, high)| {
            (
                name.to_string(),
                slice_report(&net_a, &net_b, validation, move |pair| pair.incumbent.x[24] >= low && pair.incumbent.x[24] < high),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let named_slices = |specs: &[(&str, usize, f32, f32)]| {
        specs
            .iter()
            .map(|(name, index, low, high)| {
                (
                    (*name).to_string(),
                    slice_report(&net_a, &net_b, validation, |pair| pair.incumbent.x[*index] >= *low && pair.incumbent.x[*index] < *high),
                )
            })
            .collect::<serde_json::Map<_, _>>()
    };
    let facing_size = named_slices(&[
        ("small", 5, 0.0, (1.5f32).ln()),
        ("medium", 5, (1.5f32).ln(), (2.0f32).ln()),
        ("large", 5, (2.0f32).ln(), f32::INFINITY),
    ]);
    let table_size = named_slices(&[("heads_up", 8, 0.0, 2.5 / 6.0), ("multiway", 8, 2.5 / 6.0, f32::INFINITY)]);
    let stack_depth = named_slices(&[
        ("shallow", 7, 0.0, (2.0f32).ln()),
        ("medium", 7, (2.0f32).ln(), (4.0f32).ln()),
        ("deep", 7, (4.0f32).ln(), f32::INFINITY),
    ]);
    let profile_history = named_slices(&[("sparse", 24, 0.0, 0.2), ("established", 24, 0.2, f32::INFINITY)]);
    let line_history = [
        ("unopened", slice_report(&net_a, &net_b, validation, |pair| pair.incumbent.x[10] == 0.0)),
        ("raised_preflop", slice_report(&net_a, &net_b, validation, |pair| pair.incumbent.x[10] > 0.0)),
        ("prior_postflop_call", slice_report(&net_a, &net_b, validation, |pair| pair.candidate.x[37] > 0.0)),
    ]
    .into_iter()
    .map(|(name, report)| (name.to_string(), report))
    .collect::<serde_json::Map<_, _>>();
    Ok(json!({"train_samples": train_pairs.len(), "validation_samples": validation.len(),
        "incumbent_input": 37, "candidate_input": 38, "epochs": epochs,
        "incumbent_loss": confidence(&a).0, "candidate_loss": confidence(&b).0,
        "stat_baseline_loss": confidence(&baseline).0,
        "paired_delta": {"mean": mean, "standard_error": se, "ci95": [low, high]},
        "repeated_call_river": repeated_call_report(&net_a, &net_b, validation),
        "by_street": streets, "by_opponent_confidence": confidence_slices,
        "by_facing_size": facing_size, "by_table_size": table_size,
        "by_stack_depth": stack_depth, "by_profile_history": profile_history,
        "by_line_history": line_history}))
}

fn argument(name: &str) -> Option<String> {
    let args = std::env::args().collect::<Vec<_>>();
    args.windows(2).find(|pair| pair[0] == name).map(|pair| pair[1].clone())
}

fn main() -> Result<()> {
    let cutoff = argument("--cutoff").context("usage: neural_ab --cutoff RFC3339 --seed N [--epochs N]")?;
    let validation_end = argument("--validation-end");
    let seed: u64 = argument("--seed").context("missing --seed")?.parse()?;
    let epochs: usize = argument("--epochs").map(|value| value.parse()).transpose()?.unwrap_or(10);
    let root = std::env::var("SVANBOT10_ROOT").map(std::path::PathBuf::from).unwrap_or(std::env::current_dir()?);
    let store = Store::open(&root.join("artifacts/svanbot10.db"))?;
    let exclude = store.bot_names()?;
    let mut rows = Vec::new();
    for bot in &exclude {
        rows.extend(store.recent_fit_hands(bot, 200_000)?);
    }
    rows.sort_by(|a, b| (a.ended_at.as_str(), a.hand_id.as_str()).cmp(&(b.ended_at.as_str(), b.hand_id.as_str())));
    let (train_rows, mut validation_rows) = split_rows(&rows, &cutoff);
    if let Some(end) = validation_end.as_deref() {
        validation_rows.retain(|row| row.ended_at.as_str() < end);
    }
    if train_rows.iter().any(|row| validation_rows.iter().any(|other| other.hand_id == row.hand_id)) {
        bail!("train/validation hand IDs overlap");
    }
    let (train_pairs, validation) = chronological_pairs(&rows, &cutoff, validation_end.as_deref(), &exclude)?;
    let mut report = evaluate(&train_pairs, &validation, seed, epochs)?;
    report["cutoff"] = json!(cutoff);
    report["validation_end"] = json!(validation_end);
    report["seed"] = json!(seed);
    report["train_hands"] = json!(train_rows.len());
    report["validation_hands"] = json!(validation_rows.len());
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, ended_at: &str) -> HandRow {
        HandRow {
            bot: "bot".into(),
            hand_id: id.into(),
            table_id: "table".into(),
            ended_at: ended_at.into(),
            hero_seat: None,
            hole: String::new(),
            board: String::new(),
            pot: 0,
            net: None,
            winners: String::new(),
            summary: "{}".into(),
            showdown: false,
        }
    }

    #[test]
    fn chronological_split_has_disjoint_boundaries() {
        let rows = vec![row("old", "2026-09-01T00:00:00Z"), row("boundary", "2026-09-10T00:00:00Z")];
        let (train, validation) = split_rows(&rows, "2026-09-10T00:00:00Z");
        assert_eq!(train.iter().map(|item| item.hand_id.as_str()).collect::<Vec<_>>(), ["old"]);
        assert_eq!(validation.iter().map(|item| item.hand_id.as_str()).collect::<Vec<_>>(), ["boundary"]);
    }

    #[test]
    fn paired_delta_confidence_math() {
        let (mean, se, low, high) = confidence(&[-1.0, 0.0, 1.0, 2.0]);
        assert!((mean - 0.5).abs() < 1e-12);
        assert!((se - 0.645_497_224_367_902_8).abs() < 1e-12);
        assert!(low < mean && high > mean);
    }

    #[test]
    fn evaluation_rejects_too_few_validation_samples() {
        assert!(evaluate(&[], &[], 1, 1).unwrap_err().to_string().contains("at least 300"));
    }
}
