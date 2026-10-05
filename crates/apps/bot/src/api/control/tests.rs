use super::*;

#[tokio::test]
async fn split_bot_commands_do_not_acknowledge_failed_persistence() {
    let mut shared = Shared::for_test("split-command-write-failure", &["A"]);
    Arc::get_mut(&mut shared).unwrap().config.head = true;
    let conn = rusqlite::Connection::open(shared.config.artifacts.join("svanbot10.db")).unwrap();
    conn.execute_batch("ALTER TABLE kv RENAME TO kv_hidden").unwrap();
    for command in ["pause", "stop"] {
        let response = bot_command(State(shared.clone()), Path(0), Json(json!({"command": command}))).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE, "{command} never reached the worker");
        assert_eq!(shared.bots[0].read().desired, "run", "do not display an undelivered command");
    }
}

#[tokio::test]
async fn split_bot_commands_persist_the_workers_desired_mode() {
    let mut shared = Shared::for_test("split-command-success", &["A"]);
    Arc::get_mut(&mut shared).unwrap().config.head = true;
    for (command, desired) in [("pause", "pause"), ("stop", "stop"), ("start", "run")] {
        let response = bot_command(State(shared.clone()), Path(0), Json(json!({"command": command}))).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(shared.store.get_kv(&crate::live::want_key("A")).unwrap().as_deref(), Some(desired));
        assert_eq!(shared.bots[0].read().desired, desired);
    }
}

#[tokio::test]
async fn local_bot_commands_still_apply_when_persistence_fails() {
    let shared = Shared::for_test("local-command-write-failure", &["A"]);
    let conn = rusqlite::Connection::open(shared.config.artifacts.join("svanbot10.db")).unwrap();
    conn.execute_batch("ALTER TABLE kv RENAME TO kv_hidden").unwrap();
    let response = bot_command(State(shared.clone()), Path(0), Json(json!({"command": "stop"}))).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(shared.bots[0].read().desired, "stop");
}

/// The learner acts on `start` (and its alias `run`) only. Every other training command used to be
/// stored and answered `ok`, so the dashboard's stop, automatic and rollback read as queued (#877).
#[tokio::test]
async fn a_training_command_the_learner_does_not_act_on_is_refused() {
    let shared = Shared::for_test("training-command-refused", &["A"]);
    for command in ["stop", "automatic", "rollback", ""] {
        let response = training_command(State(shared.clone()), Json(json!({"command": command}))).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{command:?}");
    }
    assert!(shared.store.get_kv(crate::LEARNER_COMMAND_KEY).unwrap().is_none(), "a refused command is not stored");
    let response = training_command(State(shared.clone()), Json(json!({"command": "start"}))).await;
    assert_eq!(response.status(), StatusCode::OK);
    let stored = shared.store.get_kv(crate::LEARNER_COMMAND_KEY).unwrap().unwrap();
    assert!(crate::pacing::operator_start_at(&stored).is_some(), "what the API accepts is what the learner reads");
}

/// A log tail whose window starts inside a multi-byte character is still a tail (#877): the read
/// used to fail on the first bytes and the panel showed nothing.
#[test]
fn a_log_tail_that_starts_inside_a_character_keeps_its_lines() {
    let dir = std::env::temp_dir().join(format!("sv10-tail-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("monitor.log");
    std::fs::write(&path, "first line with an é at its end é\nsecond line\nthird line\n").unwrap();
    // 26 bytes from the end is the second byte of the `é` that ends the first line.
    assert_eq!(tail_lines(&path, 26), vec!["second line".to_string(), "third line".to_string()]);
    let _ = std::fs::remove_dir_all(&dir);
}
