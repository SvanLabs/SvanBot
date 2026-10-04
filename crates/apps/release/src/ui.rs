//! What the shell scripts get from `exec > >(tee -a artifacts/release.log) 2>&1`: every line the
//! run prints, its children's included, goes to the console and to the log the dashboard tails.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

/// A console, and optionally a log file that sees the same lines.
#[derive(Clone, Default)]
pub struct Ui {
    log: Arc<Mutex<Option<std::fs::File>>>,
}

impl Ui {
    /// Start a fresh log (`: > artifacts/release.log`); every line from now on lands in it too.
    pub fn start_log(&self, path: &Path) -> std::io::Result<()> {
        *self.log.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::fs::File::create(path)?);
        Ok(())
    }

    /// One line. Once a log is open the script's stderr is folded into stdout, so this is the only channel.
    pub fn say(&self, line: &str) {
        let mut log = self.log.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(file) = log.as_mut() {
            let _ = writeln!(file, "{line}");
            println!("{line}");
        } else {
            println!("{line}");
        }
    }

    /// A message for stderr; with a log open it joins the stream like the shell's `2>&1`.
    pub fn complain(&self, line: &str) {
        if self.log.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
            self.say(line);
        } else {
            eprintln!("{line}");
        }
    }

    /// Run `command` with its output on this stream; its exit status (`None` when it cannot start).
    pub fn run(&self, command: &mut Command) -> Option<i32> {
        let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().ok()?;
        let pump = |stream: Box<dyn Read + Send>, ui: Ui| {
            std::thread::spawn(move || {
                for line in BufReader::new(stream).split(b'\n').map_while(Result::ok) {
                    ui.say(&String::from_utf8_lossy(&line));
                }
            })
        };
        let out = pump(Box::new(child.stdout.take()?), self.clone());
        let err = pump(Box::new(child.stderr.take()?), self.clone());
        let status = child.wait().ok();
        let _ = (out.join(), err.join());
        status.map(|s| s.code().unwrap_or(-1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_childs_two_streams_and_our_own_lines_all_reach_the_log() {
        let dir = std::env::temp_dir().join(format!("sv10-release-ui-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("release.log");
        std::fs::write(&log, "old\n").unwrap();
        let ui = Ui::default();
        ui.complain("before the log: stderr only");
        ui.start_log(&log).unwrap();
        ui.say("mine");
        ui.complain("an error line");
        assert_eq!(ui.run(Command::new("sh").args(["-c", "echo out; echo err >&2; exit 3"])), Some(3));
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(!text.contains("old") && !text.contains("before the log"), "the log starts fresh: {text}");
        for line in ["mine", "an error line", "out", "err"] {
            assert!(text.lines().any(|l| l == line), "{line} missing from {text}");
        }
        assert_eq!(ui.run(&mut Command::new("/nonexistent/program")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
