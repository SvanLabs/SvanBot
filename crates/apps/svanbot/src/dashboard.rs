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
use svanbot_runtime::fleet::{self, ArenaRun};
use svanbot_runtime::release::version_cmp;
use svanbot_runtime::slot::Slots;
use svanbot_runtime::status::Fleet;
use svanbot_runtime::store::Store;
use svanbot_strategy::policy::{Branch, Ranged, Trace};

mod page;
mod stream;

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

/// The longest request head read, and the longest wait for one.
const HEAD_LIMIT: usize = 8 * 1024;
const HEAD_WAIT: Duration = Duration::from_secs(5);

/// The answer when a store is there but cannot be read: not the caller's mistake, and not a
/// bot that has played nothing.
const INTERNAL_ERROR: u16 = 500;

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

/// The newest traced hands of one bot, newest first — what the replayer's list opens.
///
/// A bot with no store yet has played nothing, which is an empty list. A name no bot can have,
/// or a store that cannot be read, is not an empty list: one is a 404 and the other a 500, since
/// a page that answered "this bot has no hands" to every question would hide a mistyped name.
fn hands_json(config: &Config, bot: &str) -> Option<(u16, String)> {
    if !page::is_name(bot) {
        return None;
    }
    let path = config.store_of(bot);
    let store = match Store::open_reader(&path) {
        Ok(store) => store,
        Err(_) if !path.exists() => {
            return Some((200, json!({ "bot": bot, "hands": [] }).to_string()));
        }
        Err(error) => {
            return Some((
                INTERNAL_ERROR,
                json!({ "detail": format!("{bot}'s store cannot be read: {error}") }).to_string(),
            ));
        }
    };
    let hands = match store.traced_hands() {
        Ok(hands) => hands,
        Err(error) => {
            return Some((
                INTERNAL_ERROR,
                json!({ "detail": format!("{bot}'s hands cannot be read: {error}") }).to_string(),
            ));
        }
    };
    let hands: Vec<Value> = hands
        .into_iter()
        .map(|hand| {
            json!({
                "hand_id": hand.hand_id,
                "table_id": hand.table_id,
                "recorded_ms": hand.recorded_ms,
                "decisions": hand.decisions,
            })
        })
        .collect();
    Some((200, json!({ "bot": bot, "hands": hands }).to_string()))
}

/// One hand's decisions, oldest first. `None` when the store does not hold the hand at all; a hand
/// it holds with no decisions is `"traced": false`, with the note the page reads, because a hand
/// played before the trace existed is not a hand that made no decisions.
///
/// A store that is there and cannot be read is neither: it is `INTERNAL_ERROR`, the same as the
/// hands list, so a corrupt store does not read as a hand that was never played.
fn trace_json(config: &Config, route: &str) -> Option<(u16, String)> {
    let (bot, hand_id) = route.split_once('/')?;
    if !page::is_name(bot) || !page::is_name(hand_id) {
        return None;
    }
    let path = config.store_of(bot);
    let store = match Store::open_reader(&path) {
        Ok(store) => store,
        // A bot that has played nothing has no store, and no store holds no hands: the same answer
        // the hands list gives, so the two routes agree about a bot that has just started.
        Err(_) if !path.exists() => return None,
        Err(error) => {
            return Some((
                INTERNAL_ERROR,
                json!({ "detail": format!("{bot}'s store cannot be read: {error}") }).to_string(),
            ));
        }
    };
    // The store opens lazily, so a file that is not a database fails here rather than at
    // `open_reader`: a read error on a store that is there is a 500, not a hand nobody played.
    let holds = match store.holds(hand_id) {
        Ok(holds) => holds,
        Err(error) => {
            return Some((
                INTERNAL_ERROR,
                json!({ "detail": format!("{bot}'s store cannot be read: {error}") }).to_string(),
            ));
        }
    };
    if !holds {
        return None;
    }
    let traces = match store.traces(hand_id) {
        Ok(traces) => traces,
        // A decision the codec cannot read is not a hand with no trace: the hands list counts
        // that decision, so answering "no trace" here would show the page an empty hand the
        // store says it holds decisions for.
        Err(error) => {
            return Some((
                INTERNAL_ERROR,
                json!({ "detail": format!("{bot}'s trace cannot be read: {error}") }).to_string(),
            ));
        }
    };
    match traces {
        Some(traces) => {
            let decisions: Vec<Value> = traces.iter().map(trace_value).collect();
            Some((
                200,
                json!({ "bot": bot, "hand_id": hand_id, "traced": true, "decisions": decisions })
                    .to_string(),
            ))
        }
        None => Some((
            200,
            json!({
                "bot": bot,
                "hand_id": hand_id,
                "traced": false,
                "decisions": [],
                "note": "no trace — this hand was played before traces were recorded, or it has been pruned",
            })
            .to_string(),
        )),
    }
}

/// The trace as the page reads it. The street and branch are named where they are defined
/// (`Street::name`, `Branch::name`), so the page and the store cannot spell them apart.
fn trace_value(trace: &Trace) -> Value {
    let mut value = json!({
        "street": trace.street.name(),
        "branch": Branch::name(trace.branch),
        "hole": trace.hole.to_string(),
        "board": trace.board.to_string(),
    });
    if let Some(resolved) = &trace.resolved {
        let actions: Vec<String> = resolved
            .actions
            .iter()
            .map(|action| action.named())
            .collect();
        value["values"] = json!(resolved.values);
        value["strategy"] = json!(resolved.strategy);
        value["taken"] = json!(resolved.taken);
        value["actions"] = json!(actions);
        value["ranges"] = json!(ranges_value(&resolved.ranges));
    }
    if let Some(priced) = &trace.priced {
        value["equity"] = json!(priced.equity);
        value["price"] = json!(priced.price);
        value["exploit"] = json!(priced.exploit);
        value["ranges"] = json!(ranges_value(&priced.ranges));
    }
    value
}

/// Each range's seat, as the trace keeps it. A seat the spot did not carry is `null`: the page
/// says the seat is unknown rather than numbering the opponents itself, which would put a label
/// on a player the table view does not show.
fn ranges_value(ranges: &[Ranged]) -> Vec<Value> {
    ranges
        .iter()
        .map(|ranged| json!({ "seat": ranged.seat, "masses": ranged.masses }))
        .collect()
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
        for league in [fleet::League::HeadsUp, fleet::League::SixMax] {
            if let Some(latest) = runs.iter().find(|run| run.league == league)
                && latest.champion != digest
            {
                problems.push(format!(
                    "the last {} arena run played champion {}, not the champion in play: play \
                     `svanbot arena`",
                    league.name(),
                    latest.champion
                ));
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
        std::env::var("SVANBOT_DASHBOARD_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_owned());
    let server = match Server::bind(Config {
        addr: addr.clone(),
        data_dir: data_dir.clone(),
        fleet_dir,
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
