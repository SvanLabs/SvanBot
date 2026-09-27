//! Per-opponent reads (0222, 0223): every per-opponent correction the live models apply, the
//! held-out evidence that installed it (or kept it out), and the opponents it moves most.
//!
//! Three fits read opponents one by one, each fitted by the learner and installed through
//! [`crate::playerfits::PlayerFits`] only while it beats the shared model on newer data:
//! fold calibration (0214), response correction (0210) and river sizing tells (0223).

use super::*;
use sv10_core::model::ModelStore;

/// One per-opponent fit as the panel shows it; `keys` names the stored fit's gain, half-width and
/// sample-count fields.
fn fit_json(id: &str, title: &str, reads: &str, stored: Option<&Value>, keys: [&str; 3], installed: usize) -> Value {
    let [gain_key, hw_key, n_key] = keys;
    let f = |k: &str| stored.and_then(|v| v[k].as_f64());
    let evidence = match (f(gain_key), f(hw_key)) {
        (Some(g), Some(h)) => json!({"gain_mnats": g * 1000.0, "half_width_mnats": h * 1000.0, "n": f(n_key)}),
        _ => Value::Null,
    };
    json!({
        "id": id,
        "title": title,
        "reads": reads,
        "stored": stored.is_some(),
        "active": stored.and_then(|v| v["active"].as_bool()).unwrap_or(false),
        "evidence": evidence,
        "installed": installed,
    })
}

/// The panel's data: the three per-opponent fits (stored evidence and how many opponents each
/// corrects in live play), and the most-observed opponents with any correction.
pub(super) fn intel_value(
    fold: Option<&Value>,
    response: Option<&Value>,
    sizing: Option<&Value>,
    models: &ModelStore,
    top: usize,
) -> Value {
    let fits = vec![
        fit_json(
            "fold",
            "Fold calibration",
            "How often each opponent folds to our heads-up postflop bets against the street-calibrated estimate",
            fold,
            ["gain", "half_width", "n"],
            models.fold_offsets.len(),
        ),
        fit_json(
            "response",
            "Response correction",
            "Each opponent's fold, call and raise frequencies facing a bet against the neural response model",
            response,
            ["gain_facing", "half_width_facing", "n_facing"],
            models.response_ratios.len(),
        ),
        fit_json(
            "sizing",
            "River sizing tells",
            "How each opponent's river bet size tracks the strength of the hand they bet",
            sizing,
            ["gain", "half_width", "n"],
            models.size_tells.len(),
        ),
    ];
    let mut names: Vec<&String> = models.fold_offsets.keys().chain(models.response_ratios.keys()).chain(models.size_tells.keys()).collect();
    names.sort();
    names.dedup();
    let mut rows: Vec<Value> = names
        .into_iter()
        .map(|name| {
            let hands = models.players.get(name).map_or(0.0, |p| p.hands);
            json!({
                "name": name,
                "hands": hands,
                "fold_offset": models.fold_offsets.get(name),
                "response_ratio": models.response_ratios.get(name),
                "size_tell": models.size_tells.get(name),
            })
        })
        .collect();
    rows.sort_by(|a, b| b["hands"].as_f64().unwrap_or(0.0).total_cmp(&a["hands"].as_f64().unwrap_or(0.0)));
    let corrected = rows.len();
    rows.truncate(top);
    json!({"fits": fits, "corrected_opponents": corrected, "opponents": rows})
}

pub(super) async fn intel(State(s): State<Arc<Shared>>) -> Response {
    off_runtime(move || -> Result<Response, ApiError> {
        let stored = |key: &str| -> Result<Option<Value>, ApiError> {
            Ok(store_read(key, s.store.get_kv(key))?.and_then(|j| serde_json::from_str::<Value>(&j).ok()))
        };
        let fold = stored(crate::playerfold::PLAYER_FOLD_KEY)?;
        let response = stored(crate::nnresidual::NN_RESIDUAL_KEY)?;
        let sizing = stored(crate::playersize::PLAYER_SIZE_KEY)?;
        let models = s.models.read();
        Ok(Json(intel_value(fold.as_ref(), response.as_ref(), sizing.as_ref(), &models, 25)).into_response())
    })
    .await
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_report_their_evidence_and_live_install_counts() {
        let mut models = ModelStore {
            fold_offsets: Arc::new([("villain0".to_string(), -0.64f32), ("fish".to_string(), 0.3)].into_iter().collect()),
            size_tells: Arc::new([("villain0".to_string(), 0.4f32)].into_iter().collect()),
            ..Default::default()
        };
        models.players.insert("villain0".into(), PlayerStats { hands: 10_031.0, ..Default::default() });
        models.players.insert("fish".into(), PlayerStats { hands: 200.0, ..Default::default() });
        let fold = json!({"gain": 0.00447, "half_width": 0.00124, "n": 23695, "active": true, "offsets": {}});
        let sizing = json!({"gain": 0.00056, "half_width": 0.00182, "n": 2378, "active": false, "tells": {}});
        let v = intel_value(Some(&fold), None, Some(&sizing), &models, 1);
        let fits = v["fits"].as_array().unwrap();
        assert_eq!(fits.len(), 3);
        assert_eq!(fits[0]["active"], true);
        assert!((fits[0]["evidence"]["gain_mnats"].as_f64().unwrap() - 4.47).abs() < 1e-9);
        assert_eq!(fits[0]["installed"], 2);
        assert_eq!(fits[1]["stored"], false, "no response fit stored");
        assert!(fits[1]["evidence"].is_null());
        assert_eq!(fits[2]["active"], false);
        assert_eq!(fits[2]["installed"], 1);
        assert_eq!(v["corrected_opponents"], 2);
        let rows = v["opponents"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "truncated to `top`, most-observed first");
        assert_eq!(rows[0]["name"], "villain0");
        assert!((rows[0]["size_tell"].as_f64().unwrap() - 0.4).abs() < 1e-6);
        assert!(rows[0]["response_ratio"].is_null());
    }

    #[test]
    fn an_empty_store_reports_three_unstored_fits() {
        let v = intel_value(None, None, None, &ModelStore::default(), 25);
        assert!(v["fits"].as_array().unwrap().iter().all(|f| f["stored"] == false && f["installed"] == 0));
        assert_eq!(v["opponents"].as_array().unwrap().len(), 0);
    }
}
