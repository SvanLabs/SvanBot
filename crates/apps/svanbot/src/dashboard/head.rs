//! A request's head: read whole, within a limit and a wait, before the dashboard answers it.

use std::io::{BufRead, BufReader};
use std::net::TcpStream;
use std::time::Duration;

/// The longest request head read, and the longest wait for one.
const HEAD_LIMIT: usize = 8 * 1024;
pub const HEAD_WAIT: Duration = Duration::from_secs(5);

/// What came on a socket: nothing at all, a head that is not a request, or a request.
pub enum Head {
    Silent,
    Bad,
    Request(String, String),
}

/// The head on `socket`, once it has all come: its method and path, or why there are none.
pub fn head(socket: &TcpStream) -> Head {
    let mut reader = BufReader::new(socket);
    let mut first = String::new();
    let got = reader.read_line(&mut first);
    if first.is_empty() {
        return Head::Silent;
    }
    let mut read = first.len();
    while got.is_ok() {
        let mut line = String::new();
        let Ok(count) = reader.read_line(&mut line) else {
            break;
        };
        read += count;
        if count == 0 || read > HEAD_LIMIT {
            break;
        }
        if line == "\r\n" || line == "\n" {
            let mut parts = first.split_whitespace();
            if let (Some(method), Some(path)) = (parts.next(), parts.next()) {
                return Head::Request(method.to_owned(), path.to_owned());
            }
            break;
        }
    }
    Head::Bad
}
