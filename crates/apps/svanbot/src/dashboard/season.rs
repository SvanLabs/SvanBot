//! The season board on the dashboard (V11-44): the venue's leaderboard, read once a minute on a
//! thread of its own, so a slow venue never holds a page or the event stream.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use svanbot_runtime::season;
use svanbot_runtime::status::Fleet;

/// How often the venue is asked: the leaderboard's own refresh is not faster than this.
const EVERY: Duration = Duration::from_secs(60);

/// The last season board read, shared by the watcher that writes it and every page and stream
/// that reads it. `null` until the first read.
#[derive(Clone, Debug, Default)]
pub struct Season(Arc<Mutex<Value>>);

impl Season {
    /// The board as it stands.
    pub fn get(&self) -> Value {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Replaces the board.
    pub fn set(&self, board: Value) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = board;
    }

    /// Keeps the last board and says why the latest read failed, so the page shows a stale board
    /// as stale rather than dropping it.
    fn failed(&self, why: String) {
        let mut board = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if board.is_object() {
            board["error"] = json!(why);
        } else {
            *board = json!({ "error": why });
        }
    }
}

/// Reads the season from the venue once a minute for as long as the dashboard runs, marking our
/// bots by the names in `data_dir`'s statuses.
pub fn watch(season: Season, data_dir: PathBuf) {
    thread::spawn(move || {
        let mut rest = season::client();
        loop {
            let ours: Vec<String> = Fleet::read(&data_dir)
                .map(|fleet| fleet.bots.into_iter().map(|status| status.bot).collect())
                .unwrap_or_default();
            match season::read(&mut rest, &ours) {
                Ok(mut board) => {
                    board["read_ms"] = json!(crate::now_ms());
                    season.set(board);
                }
                Err(why) => season.failed(why),
            }
            thread::sleep(EVERY);
        }
    });
}
