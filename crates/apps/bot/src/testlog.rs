//! Log capture for tests: a fix whose only effect is a warning can show the warning (#347).

use std::io::Write;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Buf(Arc<Mutex<Vec<u8>>>);

impl Write for Buf {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Runs `f` and returns its result with everything it logged at any level, one line per event and
/// no colour. The subscriber is set for this thread only, so events from threads `f` starts are
/// not captured; call the function under test directly, not through the blocking pool.
pub(crate) fn capture<T>(f: impl FnOnce() -> T) -> (T, String) {
    let buf = Buf::default();
    let sink = buf.clone();
    let subscriber =
        tracing_subscriber::fmt().with_writer(move || sink.clone()).with_ansi(false).with_max_level(tracing::Level::TRACE).finish();
    let out = tracing::subscriber::with_default(subscriber, f);
    let log = String::from_utf8_lossy(&buf.0.lock().unwrap()).into_owned();
    (out, log)
}

#[cfg(test)]
mod tests {
    use super::capture;

    #[test]
    fn returns_the_result_and_every_level_logged_on_this_thread() {
        let (n, log) = capture(|| {
            tracing::info!("one");
            tracing::warn!("two {}", 2);
            tracing::debug!("three");
            7
        });
        assert_eq!(n, 7);
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(lines.len(), 3, "{log}");
        assert!(lines[1].contains("WARN") && lines[1].contains("two 2"), "{log}");
        assert!(!log.contains('\u{1b}'), "no colour codes: {log:?}");
    }

    #[test]
    fn a_quiet_closure_leaves_an_empty_log() {
        assert_eq!(capture(|| ()).1, "");
    }
}
