//! The one event stream: a fleet event whenever the fleet changes, and a tick every five seconds
//! so the page can tell a quiet fleet from a dead connection.

use std::io::{self, Write};
use std::net::TcpStream;
use std::thread;
use std::time::{Duration, Instant};

use super::{Config, fleet_now, with_clock};

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

/// Writes the event stream until the client goes away: each event is flushed as it is written,
/// and the write that fails is how the server learns the page has closed.
pub(super) fn serve(mut socket: TcpStream, config: &Config) -> io::Result<()> {
    socket.write_all(HEAD.as_bytes())?;
    socket.flush()?;
    // The last fleet sent, to tell an unchanged poll from a change.
    let mut last = None;
    let mut next_tick = Instant::now();
    loop {
        let fleet = fleet_now(config);
        if last.as_ref() != Some(&fleet) {
            write!(
                socket,
                "event: fleet\ndata: {}\n\n",
                with_clock(fleet.clone())
            )?;
            last = Some(fleet);
            next_tick = Instant::now() + TICK;
        } else if Instant::now() >= next_tick {
            write!(
                socket,
                "event: tick\ndata: {{\"server_ms\":{}}}\n\n",
                crate::now_ms()
            )?;
            next_tick = Instant::now() + TICK;
        } else {
            thread::sleep(POLL);
            continue;
        }
        socket.flush()?;
    }
}
