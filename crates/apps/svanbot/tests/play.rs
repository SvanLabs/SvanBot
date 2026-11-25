#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::process::{Command, Output};

fn play(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_svanbot"))
        .env_clear()
        .arg("play")
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn play_with_no_subcommand_says_its_usage_and_exits_2() {
    let result = play(&[]);
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("svanbot play arena"));
}

#[test]
fn an_unknown_kind_or_style_is_refused_before_anything_plays() {
    for args in [
        ["arena", "unknown", "standard", "2"],
        ["arena", "solving", "unknown", "2"],
    ] {
        let result = play(&args);
        assert_eq!(result.status.code(), Some(2));
        assert!(result.stdout.is_empty());
        let stderr = String::from_utf8_lossy(&result.stderr);
        for name in ["solving", "pricing", "standard", "tight", "loose"] {
            assert!(stderr.contains(name), "{stderr}");
        }
        assert!(!stderr.contains("record the start"), "{stderr}");
    }
}
