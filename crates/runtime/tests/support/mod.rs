//! Fixtures shared by the store's tests: a fresh temporary directory, and a bare hand.

#![allow(dead_code)]

use std::path::PathBuf;

use serde_json::json;
use svanbot_runtime::record::Hand;

/// A fresh, empty directory for one test.
pub fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("svanbot-store-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A hand with its start and result, and nothing traced.
pub fn hand(id: &str) -> Hand {
    Hand {
        hand_id: id.into(),
        table_id: "t-1".into(),
        frames: vec![
            json!({"type": "hand_start", "hand_id": id}).to_string(),
            json!({"type": "hand_result", "hand_id": id}).to_string(),
        ],
        traced: Vec::new(),
    }
}
