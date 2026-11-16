//! `svanbot dashboard`: the operator's status-first view of the fleet (V11-11).
//!
//! It reads every `*.status.json` in the data directory, serves the exported Next page, and
//! pushes the fleet over one server-sent event stream. It is loopback only: the status names
//! opponents and shows their stacks, so it is never the public TV.
//!
//! `SVANBOT_DATA` is the directory of every bot's store and status, `~/.local/share/svanbot`
//! unless set. `SVANBOT_WEB` is the exported page, `~/.local/lib/svanbot/current/web` unless set.
//! `SVANBOT_DASHBOARD_ADDR` is `127.0.0.1:8080` unless set.

use std::io;
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::thread;

use svanbot_runtime::status::Fleet;
use tiny_http::{Header, Method, Request, Response, StatusCode};
mod files;
mod stream;

/// Where the dashboard listens and what it reads.
pub struct Config {
    pub addr: String,
    pub data_dir: PathBuf,
    pub web_dir: PathBuf,
}

/// A bound dashboard server. Tests bind port 0 and ask [`Server::addr`].
pub struct Server {
    server: tiny_http::Server,
    config: Arc<Config>,
    addr: SocketAddr,
}

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
        let server = tiny_http::Server::from_listener(listener, None)
            .map_err(|error| io::Error::other(error.to_string()))?;
        Ok(Server {
            server,
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
        for request in self.server.incoming_requests() {
            let config = Arc::clone(&self.config);
            thread::spawn(move || handle(request, &config));
        }
        Ok(())
    }
}

fn handle(request: Request, config: &Config) {
    if request.method() != &Method::Get {
        let _ = request.respond(text(405, "method not allowed\n"));
        return;
    }
    let path = request.url().split('?').next().unwrap_or("/").to_owned();
    match path.as_str() {
        "/api/fleet" => {
            let _ = request.respond(json(fleet_payload(&config.data_dir)));
        }
        "/api/events" => stream::serve(request, config.data_dir.clone()),
        _ if path.starts_with("/api/") => {
            let _ =
                request.respond(json("{\"detail\":\"not found\"}".into()).with_status_code(404));
        }
        _ => files::serve(request, &config.web_dir, &path),
    }
}

/// The fleet document the page reads and the stream pushes.
fn fleet_json(fleet: Fleet) -> String {
    serde_json::json!({
        "server_ms": crate::now_ms(),
        "bots": fleet.bots,
        "problems": fleet.problems,
    })
    .to_string()
}

fn fleet_payload(data_dir: &Path) -> String {
    match Fleet::read(data_dir) {
        Ok(fleet) => fleet_json(fleet),
        Err(error) => fleet_json(Fleet {
            bots: Vec::new(),
            problems: vec![format!("data directory: {error}")],
        }),
    }
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("a literal header")
}

fn json(body: String) -> Response<io::Cursor<Vec<u8>>> {
    Response::from_data(body.into_bytes())
        .with_header(header("Content-Type", "application/json; charset=utf-8"))
        .with_header(header("Cache-Control", "no-store"))
}

fn text(status: u16, body: &str) -> Response<io::Cursor<Vec<u8>>> {
    Response::from_string(body)
        .with_status_code(StatusCode(status))
        .with_header(header("Content-Type", "text/plain; charset=utf-8"))
}

/// Runs the subcommand from the environment, for `svanbot dashboard`.
pub fn run() -> ExitCode {
    let default_dir = |name: &str, fallback: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(fallback)))
    };
    let (Some(data_dir), Some(web_dir)) = (
        default_dir("SVANBOT_DATA", ".local/share/svanbot"),
        default_dir("SVANBOT_WEB", ".local/lib/svanbot/current/web"),
    ) else {
        eprintln!("svanbot dashboard: set SVANBOT_DATA and SVANBOT_WEB, or HOME");
        return ExitCode::from(2);
    };
    let addr =
        std::env::var("SVANBOT_DASHBOARD_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_owned());

    let server = match Server::bind(Config {
        addr: addr.clone(),
        data_dir: data_dir.clone(),
        web_dir: web_dir.clone(),
    }) {
        Ok(server) => server,
        Err(error) => {
            eprintln!("svanbot dashboard: cannot listen on {addr}: {error}");
            return ExitCode::from(2);
        }
    };
    eprintln!(
        "svanbot dashboard: http://{} (data {}, web {})",
        server.addr(),
        data_dir.display(),
        web_dir.display()
    );
    match server.run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("svanbot dashboard: {error}");
            ExitCode::from(1)
        }
    }
}
