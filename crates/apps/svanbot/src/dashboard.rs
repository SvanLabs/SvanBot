//! `svanbot dashboard`: the operator's status-first view of the fleet (V11-11).
//!
//! It reads every `*.status.json` in the data directory and the slots of the install tree, serves
//! the page, and pushes the fleet over one server-sent event stream. It is loopback only unless
//! the operator opens it ([`Server::bind_open`]): the status names opponents and shows their
//! stacks and the page has no login, so it is never the public TV.
//!
//! The server and the page are our own: a `TcpListener` with one thread per connection, and
//! three hand-written files compiled into this binary ([`page`]). The box runs nothing else for
//! it.
//!
//! `SVANBOT_DATA` is the directory of every bot's store and status, `~/.local/share/svanbot`
//! unless set. `SVANBOT_LIB` is the install tree, `~/.local/lib/svanbot` unless set.
//! `SVANBOT_DASHBOARD_ADDR` is `127.0.0.1:5000` unless set, and an address beyond loopback needs
//! `SVANBOT_DASHBOARD_OPEN=yes` beside it.

use std::io::{self, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::thread;

use serde_json::{Value, json};
use svanbot_runtime::fleet::{self, ArenaRun};
use svanbot_runtime::status::Fleet;

mod hands;
mod head;
mod page;
mod reads;
mod release;
mod round_robin;
mod search;
mod season;
mod stream;

use hands::{hands_json, trace_json};
use head::{HEAD_WAIT, Head, head};
use release::release;
pub use season::Season;

/// Where the dashboard listens and what it reads.
pub struct Config {
    pub addr: String,
    pub data_dir: PathBuf,
    /// Where the fleet's files are: the champion, the proposal and the Arena's runs. Passed in
    /// rather than guessed, so the page reads what `svanbot arena` wrote even when
    /// `SVANBOT_FLEET` moves it (the binary's `fleet_dir`).
    pub fleet_dir: PathBuf,
    /// The install tree, whose slots say what is running.
    pub lib_dir: PathBuf,
    /// The season board the watcher last read from the venue (`season::watch`).
    pub season: Season,
}

impl Config {
    /// One bot's store: `data_dir/{bot}.db`. The caller has already refused a name no bot can
    /// have (`page::is_name`), since only a name reaches this path.
    pub fn store_of(&self, bot: &str) -> PathBuf {
        self.data_dir.join(format!("{bot}.db"))
    }
}

/// A bound dashboard server. Tests bind port 0 and ask [`Server::addr`].
pub struct Server {
    listener: TcpListener,
    config: Arc<Config>,
    addr: SocketAddr,
}

/// The answer when a store is there but cannot be read: not the caller's mistake, and not a
/// bot that has played nothing.
const INTERNAL_ERROR: u16 = 500;

impl Server {
    /// Binds `config.addr`, port 0 included. The dashboard is loopback only unless it is opened
    /// ([`Server::bind_open`]): its status names opponents and shows their stacks, so it is never
    /// the public TV.
    pub fn bind(config: Config) -> io::Result<Server> {
        Server::listen(config, false)
    }

    /// [`Server::bind`] for a dashboard the operator opened (`SVANBOT_DASHBOARD_OPEN=yes`): any
    /// address will do. The page has no login, so the caller asks the operator first.
    pub fn bind_open(config: Config) -> io::Result<Server> {
        Server::listen(config, true)
    }

    /// Binds `config.addr`, and refuses an address beyond loopback unless `open`.
    fn listen(config: Config, open: bool) -> io::Result<Server> {
        let listener = TcpListener::bind(&config.addr)?;
        let addr = listener.local_addr()?;
        if !open && !addr.ip().is_loopback() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "the dashboard is loopback only, not {addr}: SVANBOT_DASHBOARD_OPEN=yes opens it"
                ),
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
    let (method, path) = match head(&socket) {
        Head::Request(method, path) => (method, path),
        // A connection opened before it was needed: an answer it did not ask for would be read
        // as the page's, so it is closed unanswered.
        Head::Silent => return,
        Head::Bad => {
            let _ = respond(
                &mut socket,
                400,
                "text/plain; charset=utf-8",
                b"bad request\n",
            );
            return;
        }
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
        _ if path.starts_with("/api/reads/") => {
            let bot = path.trim_start_matches("/api/reads/");
            let (status, body) = reads::reads_json(config, bot)
                .unwrap_or((404, json!({ "detail": "not found" }).to_string()));
            respond(&mut socket, status, json_type, body.as_bytes())
        }
        _ if path.starts_with("/api/hands/") => {
            let bot = path.trim_start_matches("/api/hands/");
            let (status, body) = hands_json(config, bot)
                .unwrap_or((404, json!({ "detail": "not found" }).to_string()));
            respond(&mut socket, status, json_type, body.as_bytes())
        }
        _ if path.starts_with("/api/trace/") => {
            match trace_json(config, path.trim_start_matches("/api/trace/")) {
                Some((status, body)) => respond(&mut socket, status, json_type, body.as_bytes()),
                None => respond(&mut socket, 404, json_type, b"{\"detail\":\"not found\"}"),
            }
        }
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
    let mut fleet = Fleet::read(&config.data_dir).unwrap_or_else(|error| Fleet {
        bots: Vec::new(),
        problems: vec![format!("data directory: {error}")],
    });
    let (runs, arena_problems) = arena(&config.fleet_dir);
    fleet.problems.extend(arena_problems);
    json!({
        "bots": fleet.bots.iter().map(|status| status.to_json()).collect::<Vec<_>>(),
        "problems": fleet.problems,
        "release": release(&config.lib_dir),
        "arena": runs,
        "round_robin": round_robin::board(&config.fleet_dir),
        "season": config.season.get(),
        "search": search::search(&config.fleet_dir),
    })
}

/// The Arena's recorded runs, newest first, from the fleet, and any problem with them. A run file
/// that cannot be read is a problem the operator has to see, not a scoreboard silently missing;
/// so is a latest run that measured a champion the fleet no longer plays, since a promotion
/// changes play without changing the commit.
fn arena(fleet_dir: &Path) -> (Value, Vec<String>) {
    let fleet = fleet::Fleet::in_dir(fleet_dir);
    let runs = match fleet.arena_runs() {
        Ok(runs) => runs,
        Err(why) => {
            return (
                Value::Array(Vec::new()),
                vec![format!("the arena run was not read: {why}")],
            );
        }
    };
    let mut problems = Vec::new();
    // An unreadable champion is the live path's problem to report (`Fleet::champion`); here the
    // question is only whether each league's last run is of what is running. Each league is its
    // own board (V11-29), so a newer run of one says nothing of the other's.
    if let Ok(champion) = fleet.champion() {
        let digest = champion.digest().to_string();
        for league in fleet::League::ALL {
            let Some(latest) = runs.iter().find(|run| run.league == league) else {
                continue;
            };
            let name = league.name();
            match latest.champion.as_deref() {
                None => problems.push(format!(
                    "the last {name} arena run has no recorded champion: play `svanbot arena`",
                )),
                Some(played) if played != digest => problems.push(format!(
                    "the last {name} arena run played champion {played}, not the champion in play: play `svanbot arena`",
                )),
                Some(_) => {}
            }
        }
    }
    (
        Value::Array(runs.iter().map(ArenaRun::as_value).collect()),
        problems,
    )
}

/// The fleet document the page reads and the stream pushes: the fleet, and the server's clock.
fn fleet_json(config: &Config) -> String {
    with_clock(fleet_now(config))
}

fn with_clock(mut fleet: Value) -> String {
    fleet["server_ms"] = json!(crate::now_ms());
    fleet.to_string()
}

/// Runs the subcommand from the environment, for `svanbot dashboard`. `fleet_dir` is the binary's
/// (`crate::fleet_dir`), which knows how `SVANBOT_FLEET` moves the fleet away from the stores.
pub fn run(fleet_dir: PathBuf) -> ExitCode {
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
        std::env::var("SVANBOT_DASHBOARD_ADDR").unwrap_or_else(|_| "127.0.0.1:5000".to_owned());
    // The season board is the venue's public leaderboard; `SVANBOT_SEASON=off` keeps the
    // dashboard off the network altogether.
    let season = Season::default();
    if std::env::var("SVANBOT_SEASON").as_deref() != Ok("off") {
        season::watch(season.clone(), data_dir.clone());
    }
    // Only the word `yes` opens it: the page has no login and names opponents.
    let open = std::env::var("SVANBOT_DASHBOARD_OPEN").as_deref() == Ok("yes");
    let config = Config {
        addr: addr.clone(),
        data_dir: data_dir.clone(),
        fleet_dir,
        lib_dir,
        season,
    };
    let bound = if open {
        Server::bind_open(config)
    } else {
        Server::bind(config)
    };
    let server = match bound {
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
    if !server.addr().ip().is_loopback() {
        eprintln!("svanbot dashboard: open to the network, and the page has no login");
    }
    match server.run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("svanbot dashboard: {error}");
            ExitCode::from(1)
        }
    }
}
