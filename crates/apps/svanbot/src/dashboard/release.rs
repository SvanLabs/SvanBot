//! What is installed and where the slots point: the "what's running" panel.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use svanbot_runtime::release::version_cmp;
use svanbot_runtime::slot::Slots;

/// The install tree as the page reads it: the builds by number, and the version each slot names.
pub(super) fn release(lib_dir: &Path) -> Value {
    let slots = Slots::in_dir(lib_dir);
    let version = |slot: Option<PathBuf>| {
        slot.and_then(|dir| {
            dir.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
    };
    let mut installed: Vec<String> = std::fs::read_dir(lib_dir)
        .into_iter()
        .flatten()
        .flatten()
        // A slot is a symlink and a staged release is not installed: neither is a build.
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name != "staged")
        .collect();
    installed.sort_by(|a, b| version_cmp(a, b));
    json!({
        "current": version(slots.current()),
        "previous": version(slots.previous()),
        "installed": installed,
    })
}
