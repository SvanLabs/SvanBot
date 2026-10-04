use super::*;
use std::sync::Arc;

/// Make every kv read fail (or work again) by renaming the table under a second connection.
fn break_kv(shared: &Shared, broken: bool) {
    let conn = rusqlite::Connection::open(shared.config.artifacts.join("svanbot10.db")).unwrap();
    let sql = if broken { "ALTER TABLE kv RENAME TO kv_hidden" } else { "ALTER TABLE kv_hidden RENAME TO kv" };
    conn.execute_batch(sql).unwrap();
}

fn range_json(soft_width: f32) -> String {
    let p = sv10_core::oprange::RangeParams { soft_width, ..Default::default() };
    serde_json::json!({"active": true, "params": p, "report": {}, "fitted_at": 0.0}).to_string()
}

fn fixture(tag: &str) -> Arc<Shared> {
    let shared = Shared::for_test(tag, &["A"]);
    shared.store.put_kv(crate::RANGE_PARAMS_KEY, &range_json(0.5)).unwrap();
    shared
}

fn params_json(shared: &Shared) -> serde_json::Value {
    serde_json::to_value(&*shared.params.read()).unwrap()
}

#[test]
fn a_failed_store_read_keeps_every_installed_artifact() {
    let shared = fixture("installs-keep");
    let mut installs = Installs::after_startup(&shared.store);
    // Startup already installed the stored range model; the first refresh leaves it.
    shared.params.write().range = crate::fitted_range_params(Some(&range_json(0.5))).unwrap();
    installs.refresh(&shared);
    let before = params_json(&shared);
    let nn_before = shared.nn.read().is_some();

    break_kv(&shared, true);
    installs.refresh(&shared);
    installs.refresh(&shared);
    assert_eq!(params_json(&shared), before, "an unreadable store must not change the params");
    assert_eq!(shared.nn.read().is_some(), nn_before);
    let warnings = shared.log.lock().iter().filter(|l| l.level == "warn").count();
    assert_eq!(warnings, 1, "one warning per failure streak");

    break_kv(&shared, false);
    installs.refresh(&shared);
    assert_eq!(params_json(&shared), before, "reading the same values again changes nothing");
    assert!(shared.log.lock().iter().any(|l| l.message == "store readable again"));
}

#[test]
fn a_changed_range_model_installs_after_a_read_error() {
    let shared = fixture("installs-change");
    let mut installs = Installs::after_startup(&shared.store);
    break_kv(&shared, true);
    installs.refresh(&shared);
    break_kv(&shared, false);
    shared.store.put_kv(crate::RANGE_PARAMS_KEY, &range_json(0.3)).unwrap();
    installs.refresh(&shared);
    assert_eq!(shared.params.read().range.soft_width, 0.3);
}

#[test]
fn promoted_params_install_once() {
    let shared = fixture("installs-params");
    let mut installs = Installs::after_startup(&shared.store);
    let promoted = Params { call_margin: 0.123, ..Default::default() };
    shared.store.put_kv(PARAMS_KEY, &serde_json::to_string(&promoted).unwrap()).unwrap();
    installs.refresh(&shared);
    assert_eq!(shared.params.read().call_margin, 0.123);
    installs.refresh(&shared);
    let installs_logged = shared.log.lock().iter().filter(|l| l.message.starts_with("promoted strategy")).count();
    assert_eq!(installs_logged, 1);
}
#[test]
fn failed_response_correction_key_reads_preserve_installed_ratios() {
    for (tag, broken_key) in [("residual-key-error", crate::nnresidual::NN_RESIDUAL_KEY), ("net-key-error", NN_KEY)] {
        let shared = fixture(tag);
        let net = StoredNet {
            net: sv10_core::nn::Mlp::new(&[sv10_core::features::N_FEATURES, 48, 24, 3], 7),
            active: true,
            paired_poker_approved: true,
            training_contract: crate::neural::RESPONSE_TRAINING_CONTRACT.into(),
            val_loss: 0.5,
            baseline_loss: 0.6,
            train_samples: 1000,
            val_samples: 200,
            trained_at: 5.0,
        };
        let fit = crate::nnresidual::ResidualFit {
            net_trained_at: 5.0,
            active: true,
            ratios: [("nit".to_string(), [2.0, 0.5, 1.0])].into_iter().collect(),
            ..Default::default()
        };
        shared.store.put_kv(NN_KEY, &serde_json::to_string(&net).unwrap()).unwrap();
        shared.store.put_kv(crate::nnresidual::NN_RESIDUAL_KEY, &serde_json::to_string(&fit).unwrap()).unwrap();
        let mut installs = Installs::after_startup(&shared.store);
        installs.refresh(&shared);
        assert_eq!(shared.models.read().response_ratios.get("nit"), Some(&[2.0, 0.5, 1.0]));
        let old = shared.store.get_kv(broken_key).unwrap().unwrap();
        let writer = rusqlite::Connection::open(shared.config.artifacts.join("svanbot10.db")).unwrap();
        writer.execute("UPDATE kv SET value = x'00' WHERE key = ?1", [broken_key]).unwrap();
        assert!(shared.store.get_kv(broken_key).is_err());
        installs.refresh(&shared);
        assert_eq!(
            shared.models.read().response_ratios.get("nit"),
            Some(&[2.0, 0.5, 1.0]),
            "a failed key read must preserve the installed correction"
        );
        installs.refresh(&shared);
        assert_eq!(shared.log.lock().iter().filter(|l| l.level == "warn" && l.message.contains("store unreadable")).count(), 1);
        shared.store.put_kv(broken_key, &old).unwrap();
        let updated = crate::nnresidual::ResidualFit { ratios: [("nit".to_string(), [1.5, 0.75, 1.0])].into_iter().collect(), ..fit };
        shared.store.put_kv(crate::nnresidual::NN_RESIDUAL_KEY, &serde_json::to_string(&updated).unwrap()).unwrap();
        installs.refresh(&shared);
        assert_eq!(shared.models.read().response_ratios.get("nit"), Some(&[1.5, 0.75, 1.0]));
        assert!(shared.log.lock().iter().any(|l| l.message == "store readable again"));
    }
}

#[test]
fn malformed_records_keep_learning_and_repaired_records_install() {
    let keys = [
        crate::RANGE_PARAMS_KEY,
        NN_KEY,
        PARAMS_KEY,
        crate::foldcal::FOLD_CAL_KEY,
        crate::raisewar::RIVER_JAM_KEY,
        crate::raisewar::DEEP_CALL_KEY,
        crate::raisewar::OVERBET_CALL_KEY,
        crate::raisewar::OVERBET_SLOPE_KEY,
        crate::playerfold::PLAYER_FOLD_KEY,
        crate::playersize::PLAYER_SIZE_KEY,
        crate::nnresidual::NN_RESIDUAL_KEY,
        crate::profile::PROFILE_KEY,
        crate::HARDWARE_PROFILE_KEY,
    ];
    for (index, key) in keys.into_iter().enumerate() {
        for (variant, broken) in ["not json", "{}", "null"].into_iter().enumerate() {
            // Params deliberately accepts {} as defaults; use a wrong field type for its schema fault.
            let broken = if key == PARAMS_KEY && broken == "{}" { r#"{"call_margin":"bad"}"# } else { broken };
            let shared = fixture(&format!("installs-malformed-{index}-{variant}"));
            let net = StoredNet {
                net: sv10_core::nn::Mlp::new(&[sv10_core::features::N_FEATURES, 48, 24, 3], 7),
                active: true,
                paired_poker_approved: true,
                training_contract: crate::neural::RESPONSE_TRAINING_CONTRACT.into(),
                val_loss: 0.5,
                baseline_loss: 0.6,
                train_samples: 1000,
                val_samples: 200,
                trained_at: 5.0,
            };
            let put = |key, value: serde_json::Value| shared.store.put_kv(key, &value.to_string()).unwrap();
            put(NN_KEY, serde_json::to_value(&net).unwrap());
            put(
                crate::foldcal::FOLD_CAL_KEY,
                serde_json::to_value(crate::foldcal::FoldCalibration { shift: [0.2, 0.0, -0.7], ..Default::default() }).unwrap(),
            );
            put(
                crate::raisewar::RIVER_JAM_KEY,
                serde_json::to_value(crate::raisewar::RiverJamFit { shift: 0.07, active: true, ..Default::default() }).unwrap(),
            );
            for key in [crate::raisewar::DEEP_CALL_KEY, crate::raisewar::OVERBET_CALL_KEY] {
                put(key, serde_json::to_value(crate::raisewar::DeepCallFit { shift: 0.15, active: true, ..Default::default() }).unwrap());
            }
            put(
                crate::raisewar::OVERBET_SLOPE_KEY,
                serde_json::to_value(crate::raisewar::OverbetSlopeFit { slope: 0.06, active: true, ..Default::default() }).unwrap(),
            );
            put(
                crate::playerfold::PLAYER_FOLD_KEY,
                serde_json::to_value(crate::playerfold::PlayerFoldFit {
                    active: true,
                    offsets: [("nit".into(), -0.4)].into_iter().collect(),
                    ..Default::default()
                })
                .unwrap(),
            );
            put(
                crate::playersize::PLAYER_SIZE_KEY,
                serde_json::to_value(sv10_core::sizetell::SizeTellFit {
                    active: true,
                    tells: [("nit".into(), 0.5)].into_iter().collect(),
                    ..Default::default()
                })
                .unwrap(),
            );
            put(
                crate::nnresidual::NN_RESIDUAL_KEY,
                serde_json::to_value(crate::nnresidual::ResidualFit {
                    net_trained_at: 5.0,
                    active: true,
                    ratios: [("nit".into(), [2.0, 0.5, 1.0])].into_iter().collect(),
                    ..Default::default()
                })
                .unwrap(),
            );
            put(PARAMS_KEY, serde_json::to_value(Params { call_margin: 0.123, ..Default::default() }).unwrap());
            put(crate::HARDWARE_PROFILE_KEY, serde_json::json!({"logical_cores":8,"tuning":{"live_samples":4000,"decision_samples":1000}}));
            let mut installs = Installs::after_startup(&shared.store);
            shared.params.write().range = crate::fitted_range_params(Some(&range_json(0.5))).unwrap();
            put(crate::profile::PROFILE_KEY, serde_json::json!({"name":"quiet","live_scale":0.25,"learner_threads":2,"analyst_threads":1}));
            installs.refresh(&shared);
            let before = params_json(&shared);
            let players = crate::playerfits::PlayerFits::of(&shared.models.read());
            let saved = shared.store.get_kv(key).unwrap().unwrap();
            shared.store.put_kv(key, broken).unwrap();
            if key == crate::HARDWARE_PROFILE_KEY {
                put(
                    crate::profile::PROFILE_KEY,
                    serde_json::json!({"name":"balanced","live_scale":0.5,"learner_threads":4,"analyst_threads":2}),
                );
            }
            installs.refresh(&shared);
            installs.refresh(&shared);
            assert_eq!(params_json(&shared), before, "{key}: {broken} removed installed parameters");
            assert!(shared.nn.read().is_some(), "{key}: {broken} removed the network");
            assert_eq!(crate::playerfits::PlayerFits::of(&shared.models.read()), players, "{key}: {broken} removed per-opponent learning");
            assert_eq!(
                shared.log.lock().iter().filter(|l| l.level == "warn").count(),
                1,
                "{key}: parse failure must warn once and remain retryable"
            );
            shared.store.put_kv(key, &saved).unwrap();
            shared.store.put_kv(crate::RANGE_PARAMS_KEY, &range_json(0.3)).unwrap();
            installs.refresh(&shared);
            assert_eq!(shared.params.read().range.soft_width, 0.3, "valid repair installs");
            assert!(shared.log.lock().iter().any(|l| l.message == "store readable again"), "{key}: repaired data must clear failure state");
        }
    }
}

#[test]
fn a_valid_inactive_range_record_still_retires_the_fit() {
    let shared = fixture("installs-retire");
    let mut installs = Installs::after_startup(&shared.store);
    shared.params.write().range = crate::fitted_range_params(Some(&range_json(0.5))).unwrap();
    let mut retired: serde_json::Value = serde_json::from_str(&range_json(0.3)).unwrap();
    retired["active"] = serde_json::Value::Bool(false);
    shared.store.put_kv(crate::RANGE_PARAMS_KEY, &retired.to_string()).unwrap();
    installs.refresh(&shared);
    assert_eq!(shared.params.read().range, sv10_core::oprange::RangeParams::default());
    assert!(!installs.failing, "legitimate retirement is not a parse failure");
}

#[test]
fn a_bot_reads_its_own_slot_key_and_falls_back_to_the_champion() {
    let shared = Shared::for_test("installs-slot", &["A", "B"]);
    let mut installs = Installs::after_startup(&shared.store);
    installs.refresh(&shared);
    assert!(shared.bots.iter().all(|b| b.read().slot_params.is_none()), "no slot key: nothing changes");

    let own = serde_json::to_string(&Params { call_margin: 0.321, ..Default::default() }).unwrap();
    shared.store.put_kv(&crate::slot_params_key("B"), &own).unwrap();
    installs.refresh(&shared);
    assert!(shared.bots[0].read().slot_params.is_none());
    assert_eq!(shared.bots[1].read().slot_params.as_ref().unwrap().call_margin, 0.321);

    shared.store.put_kv(&crate::slot_params_key("B"), r#"{"call_margin":"bad"}"#).unwrap();
    installs.refresh(&shared);
    assert_eq!(shared.bots[1].read().slot_params.as_ref().unwrap().call_margin, 0.321, "malformed record keeps the incumbent");
}
