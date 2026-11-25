//! The dashboard listens beyond loopback only when it is told to open (V11-58). Its page has no
//! login and names opponents, so a network address is refused unless `SVANBOT_DASHBOARD_OPEN=yes`.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]

mod support;

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// `svanbot dashboard` over scratch folders, told to listen on `addr`. `open` is what
/// `SVANBOT_DASHBOARD_OPEN` holds, if it is set at all.
fn dashboard(name: &str, addr: &str, open: Option<&str>) -> Command {
    let dir = support::scratch(name);
    let mut command = Command::new(env!("CARGO_BIN_EXE_svanbot"));
    command
        .arg("dashboard")
        .env_clear()
        .env("SVANBOT_DATA", dir.join("data"))
        .env("SVANBOT_LIB", dir.join("lib"))
        .env("SVANBOT_FLEET", dir.join("fleet"))
        // The season board reads the venue, and this is not about it.
        .env("SVANBOT_SEASON", "off")
        .env("SVANBOT_DASHBOARD_ADDR", addr);
    if let Some(open) = open {
        command.env("SVANBOT_DASHBOARD_OPEN", open);
    }
    command
}

/// The dashboard process, killed when the test ends, however it ends.
struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Runs `command` to its end: its exit code, and what it said on stderr. A refusal ends at once. A
/// dashboard still running after ten seconds was not refused, and fails the test rather than
/// hanging it.
fn finish(mut command: Command) -> (Option<i32>, String) {
    let mut child = Running(command.stderr(Stdio::piped()).spawn().unwrap());
    let started = Instant::now();
    let code = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status.code();
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the dashboard is still running: it was not refused"
        );
        thread::sleep(Duration::from_millis(20));
    };
    let mut stderr = String::new();
    child
        .0
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    (code, stderr)
}

#[test]
fn a_network_address_is_refused_unless_the_dashboard_is_opened() {
    // Only the word `yes` opens it: unset, empty, `no` and `1` do not.
    for open in [None, Some(""), Some("no"), Some("1")] {
        let (code, stderr) = finish(dashboard("refused", "0.0.0.0:0", open));
        assert_eq!(code, Some(2), "{open:?}: {stderr}");
        assert!(stderr.contains("loopback only"), "{open:?}: {stderr}");
        assert!(
            stderr.contains("SVANBOT_DASHBOARD_OPEN=yes"),
            "the refusal names the setting: {open:?}: {stderr}"
        );
    }
}

#[test]
fn an_opened_dashboard_listens_beyond_loopback_and_serves_the_page() {
    let mut command = dashboard("opened", "0.0.0.0:0", Some("yes"));
    let mut child = Running(command.stderr(Stdio::piped()).spawn().unwrap());
    let mut stderr = BufReader::new(child.0.stderr.take().unwrap());

    // "svanbot dashboard: http://0.0.0.0:41234 (data …)": it says where it really listens.
    let mut line = String::new();
    stderr.read_line(&mut line).unwrap();
    let listening = line
        .strip_prefix("svanbot dashboard: http://")
        .unwrap_or_else(|| panic!("it did not start: {line}"));
    let addr = listening.split(' ').next().unwrap();
    assert!(addr.starts_with("0.0.0.0:"), "{line}");

    // And it says that the page is open and has no login, where the log will show it.
    let mut warning = String::new();
    stderr.read_line(&mut warning).unwrap();
    assert!(warning.contains("no login"), "{warning}");

    let port = addr.rsplit(':').next().unwrap();
    let (code, body) = support::get(format!("127.0.0.1:{port}").parse().unwrap(), "/");
    assert_eq!(code, 200);
    assert!(body.contains("<title>SvanBot</title>"), "{body}");
}
