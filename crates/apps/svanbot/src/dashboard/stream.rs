//! The one event stream: a fleet event whenever the fleet changes, and a tick every five seconds
//! so the page can tell a quiet fleet from a dead connection.

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use svanbot_runtime::status::Fleet;
use tiny_http::Request;

use super::fleet_json;

/// How often the data directory is read for a change.
const POLL: Duration = Duration::from_millis(500);
/// How often a tick proves the stream is alive.
const TICK: Duration = Duration::from_secs(5);

/// The stream's response head. The body ends when the connection closes, so `Connection: close`
/// is honest; the page's `EventSource` reconnects on its own.
const HEAD: &str = "HTTP/1.1 200 OK\r\n\
                    Content-Type: text/event-stream\r\n\
                    Cache-Control: no-cache\r\n\
                    Connection: close\r\n\
                    X-Accel-Buffering: no\r\n\
                    \r\n";

/// Writes the event stream until the client goes away. `tiny_http` buffers a normal response and
/// only flushes it at the end, which a stream never reaches, so this takes the socket writer and
/// flushes every event itself.
pub fn serve(request: Request, data_dir: PathBuf) {
    let mut writer = request.into_writer();
    if writer.write_all(HEAD.as_bytes()).is_err() || writer.flush().is_err() {
        return;
    }
    let mut stream = Stream::new(data_dir);
    let mut buf = [0u8; 4096];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => return,
            Ok(count) => {
                if writer.write_all(&buf[..count]).is_err() || writer.flush().is_err() {
                    return;
                }
            }
            Err(_) => return,
        }
    }
}

pub struct Stream {
    data_dir: PathBuf,
    /// The last fleet sent, to tell an unchanged poll from a change.
    last: Option<String>,
    pending: Vec<u8>,
    at: usize,
    next_tick: Instant,
}

impl Stream {
    pub fn new(data_dir: PathBuf) -> Stream {
        Stream {
            data_dir,
            last: None,
            pending: Vec::new(),
            at: 0,
            next_tick: Instant::now(),
        }
    }

    /// Fills `pending` with the next frame, if one is due.
    fn fill(&mut self) -> bool {
        let fleet = Fleet::read(&self.data_dir).unwrap_or_else(|error| Fleet {
            bots: Vec::new(),
            problems: vec![format!("data directory: {error}")],
        });
        let fingerprint = serde_json::to_string(&fleet).unwrap_or_default();
        if self.last.as_deref() != Some(fingerprint.as_str()) {
            self.last = Some(fingerprint);
            self.pending = format!("event: fleet\ndata: {}\n\n", fleet_json(fleet)).into_bytes();
            self.next_tick = Instant::now() + TICK;
            return true;
        }
        if Instant::now() >= self.next_tick {
            self.pending = format!(
                "event: tick\ndata: {{\"server_ms\":{}}}\n\n",
                crate::now_ms()
            )
            .into_bytes();
            self.next_tick = Instant::now() + TICK;
            return true;
        }
        false
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.at < self.pending.len() {
                let count = buf.len().min(self.pending.len() - self.at);
                buf[..count].copy_from_slice(&self.pending[self.at..self.at + count]);
                self.at += count;
                return Ok(count);
            }
            self.pending.clear();
            self.at = 0;
            if self.fill() {
                continue;
            }
            thread::sleep(POLL);
        }
    }
}
