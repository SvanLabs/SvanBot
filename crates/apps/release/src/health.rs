//! The health gate (`health_commit` and `await_health` in `rollback.sh`): an install is complete once
//! the fleet answers `/api/health` with the installed commit, not when the files land (#726).

use crate::error::{ReleaseError, Result};
use crate::identity::resolve_commit;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::{Duration, Instant};

/// Where `/api/health` is: `SV10_HEALTH_URL`, else the dashboard port (`SVANBOT_WEB_PORT`, from the
/// environment or the last such line of `.env`), else 5000.
pub fn url(root: &Path, env: &[(String, String)]) -> String {
    let get = |key: &str| env.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()).filter(|v| !v.is_empty());
    if let Some(url) = get("SV10_HEALTH_URL") {
        return url;
    }
    let from_env = get("SVANBOT_WEB_PORT");
    let from_file = || {
        let text = std::fs::read_to_string(root.join(".env")).ok()?;
        text.lines().rev().find_map(|l| {
            l.strip_prefix("SVANBOT_WEB_PORT=").filter(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())).map(str::to_string)
        })
    };
    format!("http://127.0.0.1:{}/api/health", from_env.or_else(from_file).unwrap_or_else(|| "5000".into()))
}

/// The `commit` the endpoint reports. Plain `http://` only, three seconds to connect and to read.
pub fn commit_at(url: &str) -> std::result::Result<String, String> {
    let rest = url.strip_prefix("http://").ok_or("only http:// health URLs are supported")?;
    let (host, path) = rest.split_once('/').map_or((rest, "/".to_string()), |(h, p)| (h, format!("/{p}")));
    let host = if host.contains(':') { host.to_string() } else { format!("{host}:80") };
    let addr = host.to_socket_addrs().map_err(|e| e.to_string())?.next().ok_or("no address")?;
    let timeout = Duration::from_secs(3);
    let mut stream = TcpStream::connect_timeout(&addr, timeout).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(timeout)).and(stream.set_write_timeout(Some(timeout))).map_err(|e| e.to_string())?;
    write!(stream, "GET {path} HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n\r\n").map_err(|e| e.to_string())?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply).map_err(|e| e.to_string())?;
    let (head, body) = reply.split_once("\r\n\r\n").ok_or("no HTTP reply")?;
    if head.split_whitespace().nth(1) != Some("200") {
        return Err(format!("HTTP status: {}", head.lines().next().unwrap_or("")));
    }
    let value: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    value["commit"].as_str().filter(|c| !c.is_empty()).map(str::to_string).ok_or_else(|| "health response has no commit".into())
}

/// Wait up to `SV10_HEALTH_TIMEOUT` seconds (default 240) for the fleet to serve `expected`; the
/// hot swap can take the settle window plus the wait for a mid-turn bot before the process restarts.
pub fn await_commit(root: &Path, expected: &str, env: &[(String, String)], out: &mut String) -> Result<()> {
    let setting = env.iter().find(|(k, _)| k == "SV10_HEALTH_TIMEOUT").map(|(_, v)| v.as_str());
    let timeout: u64 = match setting {
        None | Some("") => 240,
        Some(v) if v.bytes().all(|b| b.is_ascii_digit()) => v.parse().map_err(|_| timeout_setting())?,
        Some(_) => return Err(timeout_setting()),
    };
    let expected = resolve_commit(root, expected)?;
    let url = url(root, env);
    let deadline = Instant::now() + Duration::from_secs(timeout);
    let mut last = String::new();
    loop {
        if let Ok(got) = commit_at(&url) {
            if resolve_commit(root, &got).is_ok_and(|g| g == expected) {
                out.push_str(&format!("Health reports the installed commit {expected}\n"));
                return Ok(());
            }
            last = format!("; health last reported {got}");
        }
        if Instant::now() >= deadline {
            return Err(ReleaseError::Refused(format!(
                "the fleet did not answer /api/health with commit {expected} within {timeout}s{last}"
            )));
        }
        std::thread::sleep(Duration::from_secs(3));
    }
}

fn timeout_setting() -> ReleaseError {
    ReleaseError::Refused("SV10_HEALTH_TIMEOUT must be a whole number of seconds".into())
}
