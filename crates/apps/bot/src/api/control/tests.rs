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
