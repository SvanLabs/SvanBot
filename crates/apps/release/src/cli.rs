//! The command line: `sv10-release rollback <flags>` takes `scripts/rollback.sh` over flag by flag.
//! Usage lines and refusals are the script's, so a wrapper that `exec`s this binary is invisible.

use crate::error::{ReleaseError, Result};
use crate::layout::Root;
use crate::{adopt, fleet, gitops, health, identity, journal, lock, publish, restore, snapshot, space, store_format};
use std::path::PathBuf;

/// Entry point: the process exit status for `args` (without the program name).
pub fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("rollback") => rollback_main(&args[1..]),
        Some("update") => crate::update::run(&args[1..]),
        Some("release") => {
            eprintln!("{}", ReleaseError::NotBuilt("`release`".into()));
            1
        }
        _ => {
            eprintln!("usage: sv10-release release | update [--check | --rollback <commit>] | rollback <flags>");
            2
        }
    }
}

pub(crate) fn root_from_env() -> Result<Root> {
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
            if e != ReleaseError::Quiet {
                eprintln!("rollback: {e}");
            }
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
        Some("--validate-source-clean") => {
            arity(1, "--validate-source-clean")?;
            journal::sweep_scratch(root)?;
            let dirty = gitops::dirty_inputs(root.path())?;
            if dirty.is_empty() {
                return Ok(());
            }
            eprintln!("{dirty}");
            Err(ReleaseError::SourceDirty)
        }
        Some("--repair") => {
            arity(1, "--repair")?;
            if !journal::path(root).is_file() {
                out.push_str("No interrupted release swap to repair\n");
            }
            root.validate_layout()?;
            lock::acquire(root, env_of(env, "SV10_RELEASE_LOCK_FD")).map(drop)
        }
        Some("--snapshot") => {
            arity(2, "--snapshot <commit>")?;
            publish::create_snapshot(root, &args[1], env, out)
        }
        Some("--install") => {
            arity(4, "--install <binary-dir> <web-dir> <commit>")?;
            publish::install(root, &args[1], &args[2], &args[3], env, out)
        }
        Some("--preserve-unidentified") => {
            arity(1, "--preserve-unidentified")?;
            publish::preserve_unidentified(root, env, out)
        }
        Some("--await-health") => {
            arity(2, "--await-health <commit>")?;
            health::await_commit(root.path(), &args[1], env, out)
        }
        Some("--fleet-running") => {
            arity(1, "--fleet-running")?;
            if fleet::installed_bot_running(root.path()) { Ok(()) } else { Err(ReleaseError::Quiet) }
        }
        Some("--fleet-supervisors") => {
            arity(1, "--fleet-supervisors")?;
            if fleet::supervisors_running(root.path()) { Ok(()) } else { Err(ReleaseError::Quiet) }
        }
        Some("--adopt-legacy") => {
            arity(2, "--adopt-legacy <commit>")?;
            adopt::adopt_legacy(root, &args[1], env, out)
        }
        Some(_) if args.len() == 1 => restore::restore(root, &args[0], env, out),
        _ => Err(usage("<commit>")),
    }
}
