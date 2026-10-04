//! The health gate, the fleet probes and legacy adoption, against fake endpoints and real pids.

use super::fixture::Fixture;
use crate::cli::rollback;
use crate::error::ReleaseError;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::symlink;

fn args(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

/// A health endpoint that answers every request with `body` and `status`; returns its URL.
fn endpoint(status: &str, body: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/api/health", listener.local_addr().unwrap());
    let reply = format!("HTTP/1.0 {status}\r\nContent-Type: application/json\r\n\r\n{body}");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut request = [0u8; 512];
            let _ = stream.read(&mut request);
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    url
}

fn env(url: &str, extra: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut e = vec![("SV10_HEALTH_URL".to_string(), url.to_string())];
    e.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    e
}

#[test]
fn health_waits_for_the_installed_commit_and_names_what_it_saw() {
    let f = Fixture::new("health");
    let mut out = String::new();
    let good = endpoint("200 OK", &format!(r#"{{"commit":"{}"}}"#, f.commit));
    rollback(&f.root, &args(&["--await-health", &f.commit]), &env(&good, &[]), &mut out).unwrap();
    assert_eq!(out, format!("Health reports the installed commit {}\n", f.commit));
    let other = endpoint("200 OK", r#"{"commit":"HEAD"}"#);
    assert!(
        rollback(&f.root, &args(&["--await-health", &f.commit]), &env(&other, &[]), &mut String::new()).is_ok(),
        "a full name resolves to the same commit"
    );
    let stale = endpoint("200 OK", r#"{"commit":"deadbee"}"#);
    let err = rollback(&f.root, &args(&["--await-health", &f.commit]), &env(&stale, &[("SV10_HEALTH_TIMEOUT", "0")]), &mut String::new())
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        format!("the fleet did not answer /api/health with commit {} within 0s; health last reported deadbee", f.commit)
    );
    let down = endpoint("503 Service Unavailable", "{}");
    let err = rollback(&f.root, &args(&["--await-health", &f.commit]), &env(&down, &[("SV10_HEALTH_TIMEOUT", "0")]), &mut String::new())
        .unwrap_err();
    assert_eq!(err.to_string(), format!("the fleet did not answer /api/health with commit {} within 0s", f.commit));
    let err = rollback(&f.root, &args(&["--await-health", &f.commit]), &env(&good, &[("SV10_HEALTH_TIMEOUT", "soon")]), &mut String::new())
        .unwrap_err();
    assert_eq!(err.to_string(), "SV10_HEALTH_TIMEOUT must be a whole number of seconds");
}

#[test]
fn the_health_port_comes_from_the_environment_then_the_last_env_file_line() {
    let f = Fixture::new("health-port");
    let url = |env: &[(&str, &str)]| {
        crate::health::url(f.root.path(), &env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>())
    };
    assert_eq!(url(&[]), "http://127.0.0.1:5000/api/health");
    std::fs::write(f.dir.join(".env"), "SVANBOT_WEB_PORT=5001\nOTHER=1\nSVANBOT_WEB_PORT=5002\n").unwrap();
    assert_eq!(url(&[]), "http://127.0.0.1:5002/api/health");
    assert_eq!(url(&[("SVANBOT_WEB_PORT", "6000")]), "http://127.0.0.1:6000/api/health");
    assert_eq!(url(&[("SV10_HEALTH_URL", "http://h:1/x")]), "http://h:1/x");
}

#[test]
fn the_fleet_probes_answer_by_status_alone() {
    let f = Fixture::new("fleet");
    let mut out = String::new();
    let probe = |flag: &str| rollback(&f.root, &args(&[flag]), &[], &mut String::new());
    assert_eq!(probe("--fleet-running").unwrap_err(), ReleaseError::Quiet);
    assert_eq!(probe("--fleet-supervisors").unwrap_err(), ReleaseError::Quiet);
    assert_eq!(ReleaseError::Quiet.to_string(), "");
    // A bot whose executable is the installed file counts; one running something else does not.
    std::fs::create_dir_all(f.dir.join("target/release")).unwrap();
    std::fs::create_dir_all(f.dir.join("artifacts")).unwrap();
    symlink(std::env::current_exe().unwrap(), f.dir.join("target/release/sv10-bot")).unwrap();
    std::fs::write(f.dir.join("artifacts/worker-A.pid"), format!("{}\n", std::process::id())).unwrap();
    probe("--fleet-running").unwrap();
    std::fs::remove_file(f.dir.join("target/release/sv10-bot")).unwrap();
    std::fs::write(f.dir.join("target/release/sv10-bot"), "x").unwrap();
    assert_eq!(probe("--fleet-running").unwrap_err(), ReleaseError::Quiet);
    // A supervisor counts only while it is alive and runs from this checkout.
    let mut child = std::process::Command::new("sleep").arg("30").current_dir(&f.dir).spawn().unwrap();
    std::fs::write(f.dir.join("artifacts/learner-supervisor.pid"), format!("{}\n", child.id())).unwrap();
    probe("--fleet-supervisors").unwrap();
    child.kill().unwrap();
    // Killed but not yet reaped: a zombie, which must not read as a crash loop.
    for _ in 0..100 {
        if std::fs::read_to_string(format!("/proc/{}/stat", child.id()))
            .is_ok_and(|s| s.rsplit(')').next().is_some_and(|r| r.trim_start().starts_with('Z')))
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(probe("--fleet-supervisors").unwrap_err(), ReleaseError::Quiet);
    child.wait().unwrap();
    // A pid file naming a process from elsewhere does not count.
    std::fs::write(f.dir.join("artifacts/learner-supervisor.pid"), format!("{}\n", std::process::id())).unwrap();
    assert_eq!(probe("--fleet-supervisors").unwrap_err(), ReleaseError::Quiet);
    out.clear();
}

#[test]
fn a_legacy_build_is_refused_until_every_proof_is_in() {
    let f = Fixture::new("adopt");
    let bin = f.dir.join("target/release");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(f.dir.join("artifacts")).unwrap();
    let url = endpoint("200 OK", &format!(r#"{{"commit":"{}"}}"#, f.commit));
    let adopt = || {
        super::fixture::rollback_retrying(&f.root, &args(&["--adopt-legacy", &f.commit]), &env(&url, &[]), &mut String::new())
            .unwrap_err()
            .to_string()
    };
    // The release log must name the commit before anything else is looked at.
    assert_eq!(adopt(), format!("release log does not identify legacy commit {}", f.commit));
    std::fs::write(f.dir.join("artifacts/releases.log"), format!("2026-10-04T10:00:00+00:00 {} first\n", f.commit)).unwrap();
    assert_eq!(adopt(), "legacy required binary missing or non-executable: sv10-bot");
    for name in crate::binaries::REQUIRED {
        Fixture::program(&bin.join(name), &format!("{name} 10.0.1 {}", f.commit));
    }
    assert_eq!(adopt(), "sv10-bot is not an unmarked legacy binary");
    for name in crate::binaries::REQUIRED {
        Fixture::program(&bin.join(name), &format!("{name} 10.0.1"));
    }
    assert_eq!(adopt(), "no running process matches the installed legacy sv10-bot inode");
    std::fs::write(bin.join(crate::identity::MARKER), "x\n").unwrap();
    assert_eq!(adopt(), "installed commit marker already exists");
}
