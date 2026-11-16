//! `svanbot dashboard`: the operator's status-first view of the fleet (V11-11).
//!
//! It reads every `*.status.json` in the data directory and the slots of the install tree, serves
//! the page, and pushes the fleet over one server-sent event stream. It is loopback only: the
//! status names opponents and shows their stacks, so it is never the public TV.
//!
//! The server and the page are our own: a `TcpListener` with one thread per connection, and
//! three hand-written files compiled into this binary ([`page`]). The box runs nothing else for
//! it.
//!
//! `SVANBOT_DATA` is the directory of every bot's store and status, `~/.local/share/svanbot`
//! unless set. `SVANBOT_LIB` is the install tree, `~/.local/lib/svanbot` unless set.
//! `SVANBOT_DASHBOARD_ADDR` is `127.0.0.1:8080` unless set.

use std::io::{self, BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use svanbot_runtime::release::version_cmp;
use svanbot_runtime::slot::Slots;
use svanbot_runtime::status::Fleet;

mod page;
mod stream;

/// Where the dashboard listens and what it reads.
pub struct Config {
    pub addr: String,
    pub data_dir: PathBuf,
    /// The install tree, whose slots say what is running.
    pub lib_dir: PathBuf,
}

/// A bound dashboard server. Tests bind port 0 and ask [`Server::addr`].
pub struct Server {
    listener: TcpListener,
    config: Arc<Config>,
    addr: SocketAddr,
}

/// The longest request head read, and the longest wait for one.
const HEAD_LIMIT: usize = 8 * 1024;
const HEAD_WAIT: Duration = Duration::from_secs(5);

impl Server {
    /// Binds `config.addr`, port 0 included. The dashboard is loopback only: its status names
    /// opponents and shows their stacks, so it is never the public TV.
    pub fn bind(config: Config) -> io::Result<Server> {
        let listener = TcpListener::bind(&config.addr)?;
        let addr = listener.local_addr()?;
        if !addr.ip().is_loopback() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("the dashboard is loopback only, not {addr}"),
            ));
        }
        Ok(Server {
            listener,
            config: Arc::new(config),
            addr,
        })
    }

    /// The address actually bound.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Serves until the process ends. A slow stream holds its own thread; the accept loop never
    /// waits on it.
    pub fn run(self) -> io::Result<()> {
        for socket in self.listener.incoming() {
            let Ok(socket) = socket else { continue };
            let config = Arc::clone(&self.config);
            thread::spawn(move || handle(socket, &config));
        }
        Ok(())
    }
}

/// Reads one request and answers it. Every answer closes the connection.
fn handle(mut socket: TcpStream, config: &Config) {
    let _ = socket.set_read_timeout(Some(HEAD_WAIT));
    let Some((method, path)) = request_line(&socket) else {
        let _ = respond(
            &mut socket,
            400,
            "text/plain; charset=utf-8",
            b"bad request\n",
        );
        return;
    };
    if method != "GET" {
        let _ = respond(
            &mut socket,
            405,
            "text/plain; charset=utf-8",
            b"method not allowed\n",
        );
        return;
    }
    let path = path.split('?').next().unwrap_or("/");
    let json_type = "application/json; charset=utf-8";
    let _ = match path {
        "/api/fleet" => respond(&mut socket, 200, json_type, fleet_json(config).as_bytes()),
        "/api/events" => stream::serve(socket, config),
        _ if path.starts_with("/api/") => {
            respond(&mut socket, 404, json_type, b"{\"detail\":\"not found\"}")
        }
        _ => match page::file(path) {
            Some((kind, bytes)) => respond(&mut socket, 200, kind, bytes),
            None => respond(
                &mut socket,
                404,
                "text/plain; charset=utf-8",
                b"not found\n",
            ),
        },
    };
}

/// The method and the path of the request on `socket`, once its whole head has come.
fn request_line(socket: &TcpStream) -> Option<(String, String)> {
    let mut reader = BufReader::new(socket);
    let mut first = String::new();
    reader.read_line(&mut first).ok()?;
    let mut read = first.len();
    loop {
        let mut line = String::new();
        let count = reader.read_line(&mut line).ok()?;
        read += count;
        if count == 0 || read > HEAD_LIMIT {
            return None;
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
    }
    let mut parts = first.split_whitespace();
    Some((parts.next()?.to_owned(), parts.next()?.to_owned()))
}

/// Writes one whole response.
fn respond(socket: &mut TcpStream, status: u16, kind: &str, body: &[u8]) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    write!(
        socket,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    socket.write_all(body)?;
    socket.flush()
}

/// The fleet as it stands, without the clock: what the stream compares to see a change.
fn fleet_now(config: &Config) -> Value {
    let fleet = Fleet::read(&config.data_dir).unwrap_or_else(|error| Fleet {
        bots: Vec::new(),
        problems: vec![format!("data directory: {error}")],
    });
    json!({
        "bots": fleet.bots.iter().map(|status| status.to_json()).collect::<Vec<_>>(),
        "problems": fleet.problems,
        "release": release(&config.lib_dir),
    })
}

/// The fleet document the page reads and the stream pushes: the fleet, and the server's clock.
fn fleet_json(config: &Config) -> String {
    with_clock(fleet_now(config))
}

fn with_clock(mut fleet: Value) -> String {
    fleet["server_ms"] = json!(crate::now_ms());
    fleet.to_string()
}

/// What is installed and where the slots point: the "what's running" panel.
fn release(lib_dir: &Path) -> Value {
    let slots = Slots::in_dir(lib_dir);
    let version = |slot: Option<PathBuf>| {
        slot.and_then(|dir| {
            dir.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
    };
    let mut installed: Vec<String> = std::fs::read_dir(lib_dir)
        .into_iter()
        .flatten()
        .flatten()
        // A slot is a symlink and a staged release is not installed: neither is a build.
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name != "staged")
        .collect();
    installed.sort_by(|a, b| version_cmp(a, b));
    json!({
        "current": version(slots.current()),
        "previous": version(slots.previous()),
        "installed": installed,
    })
}

/// Runs the subcommand from the environment, for `svanbot dashboard`.
pub fn run() -> ExitCode {
    let default_dir = |name: &str, fallback: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(fallback)))
    };
    let (Some(data_dir), Some(lib_dir)) = (
        default_dir("SVANBOT_DATA", ".local/share/svanbot"),
        default_dir("SVANBOT_LIB", ".local/lib/svanbot"),
    ) else {
        eprintln!("svanbot dashboard: set SVANBOT_DATA and SVANBOT_LIB, or HOME");
        return ExitCode::from(2);
    };
    let addr =
        std::env::var("SVANBOT_DASHBOARD_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_owned());
    let server = match Server::bind(Config {
        addr: addr.clone(),
        data_dir: data_dir.clone(),
        lib_dir,
    }) {
        Ok(server) => server,
        Err(error) => {
            eprintln!("svanbot dashboard: cannot listen on {addr}: {error}");
            return ExitCode::from(2);
        }
    };
    eprintln!(
        "svanbot dashboard: http://{} (data {})",
        server.addr(),
        data_dir.display()
    );
    match server.run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("svanbot dashboard: {error}");
            ExitCode::from(1)
        }
    }
}
