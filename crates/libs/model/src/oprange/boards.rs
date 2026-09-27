//! Per-board combo strengths and percentiles, cached across threads; exact (tables) or Monte Carlo mode.

use super::*;

/// Per-board strengths plus their board-wide percentiles, shared by every thread.
pub struct BoardInfo {
    /// Strength of every combo on the board (higher is stronger).
    pub strength: Arc<Vec<f32>>,
    /// Each combo's strength percentile among all live combos on the board.
    pub pct: Vec<f32>,
}

/// Shared across every thread; lookups take a read lock so simulation threads do not queue
/// behind each other on cache hits.
fn board_cache() -> &'static RwLock<HashMap<CardMask, Arc<BoardInfo>>> {
    static C: std::sync::OnceLock<RwLock<HashMap<CardMask, Arc<BoardInfo>>>> = std::sync::OnceLock::new();
    C.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Per-combo strength on a board (higher = stronger). Preflop uses the class ordering.
pub fn board_strengths(board: &[Card]) -> Arc<Vec<f32>> {
    board_info(board).strength.clone()
}

thread_local! {
    static EXACT_STRENGTHS: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// Run `f` with flop/turn board strengths exact (true, the default: precomputed tables, else
/// computed on demand) or by the older Monte Carlo blend (false) on this thread. Decisions set it
/// from `Params::exact_strengths`; simulated archetypes use the thread default.
pub fn with_strength_mode<T>(exact: bool, f: impl FnOnce() -> T) -> T {
    let prev = EXACT_STRENGTHS.with(|c| c.replace(exact));
    let out = f();
    EXACT_STRENGTHS.with(|c| c.set(prev));
    out
}

/// Cached strengths and percentiles for `board` in the thread's strength mode.
pub fn board_info(board: &[Card]) -> Arc<BoardInfo> {
    let exact = EXACT_STRENGTHS.with(|c| c.get()) && (board.len() == 3 || board.len() == 4);
    let key = board.iter().fold(0u64, |m, c| m | c.bit()) | ((board.len() as u64) << 60) | ((exact as u64) << 59);
    if let Some(v) = board_cache().read().unwrap_or_else(|e| e.into_inner()).get(&key).cloned() {
        return v;
    }
    let strength: Vec<f32> = if board.is_empty() {
        preflop::table().combo_percentile.iter().map(|p| 1.0 - p).collect()
    } else if exact {
        sv10_equity::tables::loaded(board.len())
            .and_then(|t| t.lookup(board))
            .unwrap_or_else(|| sv10_equity::equity::exact_strengths(board))
    } else {
        let mut rng = SmallRng::seed_from_u64(key);
        let samples = if board.len() == 3 { 48 } else { 64 };
        combo_strengths(board, samples, &mut rng)
    };
    let pct = global_pct(&strength);
    let info = Arc::new(BoardInfo { strength: Arc::new(strength), pct });
    let mut c = board_cache().write().unwrap_or_else(|e| e.into_inner());
    // ~11 KB per board: 16k boards is ~180 MB, far below free RAM, and clears 8x less often.
    if c.len() > 16_384 {
        c.clear();
    }
    c.insert(key, info.clone());
    info
}
