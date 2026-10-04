//! The command line: `sv10-release rollback <flags>` takes `scripts/rollback.sh` over flag by flag.
//! Usage lines and refusals are the script's, so a wrapper that `exec`s this binary is invisible.

use crate::error::{ReleaseError, Result};
use crate::layout::Root;
use crate::{identity, snapshot, space, store_format};
use std::path::PathBuf;

/// Entry point: the process exit status for `args` (without the program name).
pub fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("rollback") => rollback_main(&args[1..]),
        Some(surface @ ("release" | "update")) => {
            let prefix = if surface == "update" { "update: " } else { "" };
            eprintln!("{prefix}{}", ReleaseError::NotBuilt(format!("`{surface}`")));
            1
        }
        _ => {
            eprintln!("usage: sv10-release release | update [--check | --rollback <commit>] | rollback <flags>");
            2
        }
    }
}

fn root_from_env() -> Result<Root> {
    let raw = match std::env::var_os("SV10_RELEASE_ROOT").filter(|v| !v.is_empty()) {
        Some(r) => PathBuf::from(r),
        None => std::env::current_dir().map_err(|e| ReleaseError::Io(format!("cannot read the working directory: {e}")))?,
    };
    Root::open(&raw, std::env::var_os("HOME").map(PathBuf::from).as_deref())
}

fn rollback_main(args: &[String]) -> i32 {
    let mut out = String::new();
    let result = root_from_env().and_then(|root| rollback(&root, args, &std::env::vars().collect::<Vec<_>>(), &mut out));
    print!("{out}");
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("rollback: {e}");
            1
        }
    }
}

fn usage(tail: &str) -> ReleaseError {
    ReleaseError::Usage(format!("usage: scripts/rollback.sh {tail}"))
}

fn env_of<'a>(env: &'a [(String, String)], key: &str) -> Option<&'a str> {
    env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

/// `scripts/rollback.sh <args>` against `root`; output lines are appended to `out`.
pub fn rollback(root: &Root, args: &[String], env: &[(String, String)], out: &mut String) -> Result<()> {
    let arity = |n: usize, tail: &str| if args.len() == n { Ok(()) } else { Err(usage(tail)) };
    match args.first().map(String::as_str) {
        Some("--validate-layout") => {
            arity(1, "--validate-layout")?;
            root.validate_layout()
        }
        Some("--verify") => {
            arity(2, "--verify <commit>")?;
            let commit = identity::resolve_commit(root.path(), &args[1])?;
            snapshot::verify(root, &commit)?;
            out.push_str(&format!("Verified release snapshot {commit}\n"));
            Ok(())
        }
        Some("--check-space") => {
            arity(1, "--check-space")?;
            let line = space::check(root.path(), env_of(env, "SV10_MIN_FREE_MB"))?;
            out.push_str(&format!("{line}\n"));
            Ok(())
        }
        Some("--data-format") => {
            arity(2, "--data-format <commit>")?;
            let commit = identity::resolve_commit(root.path(), &args[1])?;
            out.push_str(&format!("{} {}\n", store_format::of_build(root.path(), &commit), store_format::of_store(root.path())));
            Ok(())
        }
        Some("--installed-commit") => {
            arity(1, "--installed-commit")?;
            let commit = identity::installed_commit(root.path())?.ok_or(ReleaseError::NoInstalledCommit)?;
            out.push_str(&format!("{commit}\n"));
            Ok(())
        }
        Some(
            flag @ ("--validate-source-clean"
            | "--snapshot"
            | "--await-health"
            | "--fleet-running"
            | "--fleet-supervisors"
            | "--repair"
            | "--preserve-unidentified"
            | "--adopt-legacy"
            | "--install"),
        ) => Err(ReleaseError::NotBuilt(format!("`rollback {flag}`"))),
        Some(_) if args.len() == 1 => Err(ReleaseError::NotBuilt("`rollback <commit>`".into())),
        _ => Err(usage("<commit>")),
    }
}
