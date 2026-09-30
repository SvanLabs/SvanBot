//! Precomputed exact board-strength tables for flops and turns.
//!
//! Strengths depend on a board only up to a relabelling of suits, so one row per suit-canonical
//! board is stored: 1,755 flops and 16,432 turns, 1,326 `f32` each (9 MB and 87 MB). A lookup
//! maps the board to its canonical form with a suit permutation and reads the row through the
//! same permutation of combo indices.
//!
//! File format (little-endian): magic `SV10TBL1`, `u32` board length, `u32` board count, `u32`
//! combo count, `u64` canonical board masks (sorted), `f32` rows, then a `u64` FNV-1a checksum of
//! every preceding byte. A file that fails its checksum is ignored (strengths are then computed on
//! demand, with identical values).
//!
//! The rows (99% of the file) are not copied: [`Table::read`] maps the file read-only and shared
//! (`sv10-mmap`, 0228), so every process that loads a table (fleet, workers, learner, analyst, tools,
//! test binaries) reads the same page-cache copy instead of holding a private 96 MB heap copy. The
//! keys are copied (they start at byte 20, not 8-byte aligned; 130 KB for turns). Tables are replaced
//! only by write-then-rename ([`Table::write`]), which a live mapping survives.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use sv10_cards::cards::{Card, CardMask};
use sv10_cards::range::{NUM_COMBOS, combo_index, combos};

const MAGIC: &[u8; 8] = b"SV10TBL1";

/// All 24 permutations of the four suits.
fn suit_perms() -> &'static [[u8; 4]; 24] {
    static P: OnceLock<[[u8; 4]; 24]> = OnceLock::new();
    P.get_or_init(|| {
        let mut out = [[0u8; 4]; 24];
        let mut k = 0;
        for a in 0..4u8 {
            for b in 0..4u8 {
                for c in 0..4u8 {
                    for d in 0..4u8 {
                        if a != b && a != c && a != d && b != c && b != d && c != d {
                            out[k] = [a, b, c, d];
                            k += 1;
                        }
                    }
                }
            }
        }
        out
    })
}

fn permute(c: Card, p: &[u8; 4]) -> Card {
    Card(p[c.suit() as usize] * 13 + c.rank())
}

/// `combo_map()[p][i]`: index of combo `i` after applying suit permutation `p`.
fn combo_map() -> &'static Vec<[u16; NUM_COMBOS]> {
    static M: OnceLock<Vec<[u16; NUM_COMBOS]>> = OnceLock::new();
    M.get_or_init(|| {
        suit_perms()
            .iter()
            .map(|p| {
                let mut m = [0u16; NUM_COMBOS];
                for (i, &(a, b)) in combos().cards.iter().enumerate() {
                    // `p` is a permutation of the four suits and `a != b`, so the permuted pair is
                    // still two distinct cards and always has a combo.
                    m[i] = combo_index(permute(a, p), permute(b, p)).expect("a relabelled combo is a combo") as u16;
                }
                m
            })
            .collect()
    })
}

/// Canonical mask of a board (smallest mask over all suit relabellings) and a permutation index
/// that produces it.
pub fn canonical(board: &[Card]) -> (CardMask, usize) {
    let mut best = (u64::MAX, 0);
    for (k, p) in suit_perms().iter().enumerate() {
        let m = board.iter().fold(0u64, |m, &c| m | permute(c, p).bit());
        if m < best.0 {
            best = (m, k);
        }
    }
    best
}

/// Every suit-canonical board of `len` cards, sorted by mask.
pub fn canonical_boards(len: usize) -> Vec<CardMask> {
    let mut out = std::collections::BTreeSet::new();
    let mut idx: Vec<u8> = (0..len as u8).collect();
    loop {
        let board: Vec<Card> = idx.iter().map(|&i| Card(i)).collect();
        out.insert(canonical(&board).0);
        let mut k = len;
        while k > 0 && idx[k - 1] as usize == 52 - len + k - 1 {
            k -= 1;
        }
        if k == 0 {
            break;
        }
        idx[k - 1] += 1;
        for m in k..len {
            idx[m] = idx[m - 1] + 1;
        }
    }
    out.into_iter().collect()
}

/// Cards of a mask, lowest index first.
pub fn cards_of(mask: CardMask) -> Vec<Card> {
    (0..52u8).filter(|i| mask & (1u64 << i) != 0).map(Card).collect()
}

/// Board-strength table: sorted canonical board keys and, per key, one strength per combo
/// (file format `SV10TBL1`: header, keys, rows, FNV-1a checksum).
pub struct Table {
    keys: Vec<CardMask>,
    rows: Rows,
}

/// Where a table's strengths live: built in memory, or viewed in place in a shared file mapping.
enum Rows {
    Owned(Vec<f32>),
    Mapped { map: sv10_mmap::Mmap, offset: usize, count: usize },
}

impl Rows {
    fn get(&self) -> &[f32] {
        match self {
            Rows::Owned(v) => v,
            // Bounds, alignment and endianness were checked when the table was read.
            Rows::Mapped { map, offset, count } => map.f32s(*offset, *count).unwrap_or(&[]),
        }
    }
}

impl Table {
    /// A table from sorted canonical `keys` and `keys.len() × 1326` strengths.
    pub fn new(keys: Vec<CardMask>, rows: Vec<f32>) -> Table {
        assert_eq!(rows.len(), keys.len() * NUM_COMBOS);
        Table { keys, rows: Rows::Owned(rows) }
    }

    /// Whether the strengths are a shared file mapping rather than a private copy.
    pub fn is_mapped(&self) -> bool {
        matches!(self.rows, Rows::Mapped { .. })
    }

    /// Strengths of every combo on `board`, in the board's own suit labelling.
    pub fn lookup(&self, board: &[Card]) -> Option<Vec<f32>> {
        let mut out = vec![0f32; NUM_COMBOS];
        self.lookup_into(board, &mut out).then_some(out)
    }

    /// [`lookup`](Table::lookup) into a caller's buffer of 1,326 values; false when the board is not
    /// in the table.
    pub fn lookup_into(&self, board: &[Card], out: &mut [f32]) -> bool {
        let (key, perm) = canonical(board);
        let Ok(row) = self.keys.binary_search(&key) else { return false };
        let r = &self.rows.get()[row * NUM_COMBOS..(row + 1) * NUM_COMBOS];
        let map = &combo_map()[perm];
        for (o, &m) in out[..NUM_COMBOS].iter_mut().zip(map.iter()) {
            *o = r[m as usize];
        }
        true
    }

    /// Write atomically (temp file then rename) with header and checksum.
    pub fn write(&self, path: &Path, board_len: usize) -> std::io::Result<()> {
        let mut buf = Vec::with_capacity(20 + self.keys.len() * 8 + self.rows.get().len() * 4 + 8);
        buf.extend_from_slice(MAGIC);
        buf.extend_from_slice(&(board_len as u32).to_le_bytes());
        buf.extend_from_slice(&(self.keys.len() as u32).to_le_bytes());
        buf.extend_from_slice(&(NUM_COMBOS as u32).to_le_bytes());
        for k in &self.keys {
            buf.extend_from_slice(&k.to_le_bytes());
        }
        for v in self.rows.get() {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        let sum = fnv1a(&buf);
        buf.extend_from_slice(&sum.to_le_bytes());
        // The staging file is this call's alone (#581): a fixed name is shared by two builds racing on
        // one path, so one truncates or interleaves with the other's bytes and the rename then puts the
        // mixture under the table's name — or simply fails, because the other writer renamed the shared
        // staging file out from under it. Same rule as `sv10_rt::write_atomic_mode` (#501).
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let staging = format!("{}.{}.tmp", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        let tmp = path.with_extension(staging);
        let written = std::fs::write(&tmp, &buf).and_then(|()| std::fs::rename(&tmp, path));
        if written.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        written
    }

    /// Read and verify magic, checksum, board length and shape. The rows stay in a shared read-only
    /// mapping of the file; if the file cannot be mapped (or the target is big-endian) they are
    /// decoded into memory instead, with identical values.
    pub fn read(path: &Path, board_len: usize) -> Result<Table, String> {
        let fail = |e: std::io::Error| format!("{}: {e}", path.display());
        match sv10_mmap::Mmap::open(path) {
            Ok(map) => {
                let (keys, offset, count) = Table::parse(map.bytes(), path, board_len)?;
                match map.f32s(offset, count) {
                    Some(_) => Ok(Table { keys, rows: Rows::Mapped { map, offset, count } }),
                    None => Ok(Table { keys, rows: Rows::Owned(decode_rows(&map.bytes()[offset..offset + count * 4])) }),
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(fail(e)),
            Err(_) => {
                let buf = std::fs::read(path).map_err(fail)?;
                let (keys, offset, count) = Table::parse(&buf, path, board_len)?;
                Ok(Table { keys, rows: Rows::Owned(decode_rows(&buf[offset..offset + count * 4])) })
            }
        }
    }

    /// Verify a table file's bytes; returns the keys and where the rows start and how many floats
    /// they hold.
    fn parse(buf: &[u8], path: &Path, board_len: usize) -> Result<(Vec<CardMask>, usize, usize), String> {
        if buf.len() < 28 || &buf[..8] != MAGIC {
            return Err(format!("{}: not a board table", path.display()));
        }
        let (body, tail) = buf.split_at(buf.len() - 8);
        if fnv1a(body) != u64::from_le_bytes(tail.try_into().unwrap()) {
            return Err(format!("{}: checksum mismatch", path.display()));
        }
        let u32_at = |o: usize| u32::from_le_bytes(body[o..o + 4].try_into().unwrap()) as usize;
        let (len, count, ncombos) = (u32_at(8), u32_at(12), u32_at(16));
        if len != board_len || ncombos != NUM_COMBOS || body.len() != 20 + count * 8 + count * NUM_COMBOS * 4 {
            return Err(format!("{}: unexpected shape", path.display()));
        }
        let keys = body[20..20 + count * 8].as_chunks::<8>().0.iter().map(|c| u64::from_le_bytes(*c)).collect();
        Ok((keys, 20 + count * 8, count * NUM_COMBOS))
    }
}

/// Little-endian `f32`s from bytes.
fn decode_rows(bytes: &[u8]) -> Vec<f32> {
    bytes.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect()
}

/// FNV-1a 64-bit over every byte (no dependency; ~0.1 s per 90 MB).
pub fn fnv1a(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// File name of the flop (3-card) or turn (4-card) table in `dir`.
pub fn table_path(dir: &Path, board_len: usize) -> PathBuf {
    dir.join(format!("strengths-{}.sv10tbl", if board_len == 3 { "flop" } else { "turn" }))
}

/// Directory holding the tables: `SV10_TABLES_DIR`, else `artifacts/tables` under `SVANBOT10_ROOT`
/// or the current directory.
pub fn tables_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("SV10_TABLES_DIR") {
        return PathBuf::from(d);
    }
    if let Some(root) = std::env::var_os("SVANBOT10_ROOT") {
        return PathBuf::from(root).join("artifacts").join("tables");
    }
    let cwd = PathBuf::from(".").join("artifacts").join("tables");
    if cwd.is_dir() {
        return cwd;
    }
    // A tool started from another directory still finds the repo's tables: binaries live in
    // `<root>/target/<profile-dir>/release/` or `<root>/target/release/` (without this, every board's
    // strengths were silently recomputed, ~15x slower sims).
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.ancestors().find(|a| a.join("artifacts").join("tables").is_dir()).map(|a| a.join("artifacts").join("tables")))
        .unwrap_or(cwd)
}

/// The loaded table for flops (3) or turns (4), if a valid file exists. Loaded once per process
/// (plain file read: never uses rayon inside the lazy initializer, see 0047).
pub fn loaded(board_len: usize) -> Option<&'static Table> {
    static FLOP: OnceLock<Option<Table>> = OnceLock::new();
    static TURN: OnceLock<Option<Table>> = OnceLock::new();
    let cell = match board_len {
        3 => &FLOP,
        4 => &TURN,
        _ => return None,
    };
    cell.get_or_init(|| match Table::read(&table_path(&tables_dir(), board_len), board_len) {
        Ok(t) => Some(t),
        Err(e) => {
            if !e.contains("No such file") {
                eprintln!("board tables unavailable: {e}");
            }
            None
        }
    })
    .as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sv10_cards::cards::parse_cards;

    #[test]
    fn canonical_board_counts() {
        assert_eq!(canonical_boards(3).len(), 1_755);
    }

    #[test]
    fn lookup_through_suit_permutation_matches_direct_computation() {
        let boards = [vec!["Qd", "8s", "3c"], vec!["9h", "8h", "2h"], vec!["Ks", "Kd", "4s", "7c"]];
        for b in boards {
            let board = parse_cards(&b).unwrap();
            let (key, _) = canonical(&board);
            let canon = cards_of(key);
            let table = Table::new(vec![key], crate::equity::exact_strengths(&canon));
            let direct = crate::equity::exact_strengths(&board);
            let via = table.lookup(&board).unwrap();
            for i in 0..NUM_COMBOS {
                assert!((via[i] - direct[i]).abs() < 1e-5, "board {b:?} combo {i}: {} vs {}", via[i], direct[i]);
            }
        }
    }

    #[test]
    fn file_round_trip_and_corruption_detection() {
        let dir = std::env::temp_dir().join(format!("sv10-tables-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let board = parse_cards(&["Qd", "8s", "3c"]).unwrap();
        let (key, _) = canonical(&board);
        let t = Table::new(vec![key], crate::equity::exact_strengths(&cards_of(key)));
        let path = table_path(&dir, 3);
        t.write(&path, 3).unwrap();
        let back = Table::read(&path, 3).unwrap();
        assert!(back.is_mapped());
        assert_eq!(back.lookup(&board), t.lookup(&board));
        let mut buf = vec![0f32; NUM_COMBOS];
        assert!(back.lookup_into(&board, &mut buf));
        assert_eq!(Some(buf), t.lookup(&board));
        let other = parse_cards(&["Ad", "Kd", "2c"]).unwrap();
        assert!(!back.lookup_into(&other, &mut [0f32; NUM_COMBOS]));
        drop(back);
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[100] ^= 1;
        std::fs::write(&path, bytes).unwrap();
        assert!(Table::read(&path, 3).err().unwrap().contains("checksum"));
    }

    /// Two table builds racing on one path must still leave one whole table (#581).
    ///
    /// `write` stages through one fixed name, so two writers share it and one can truncate or
    /// interleave with the other's bytes; the rename that follows then puts the mixture under the
    /// table's real name. That is the class `sv10_rt::write_atomic_mode` was fixed for (#501). The
    /// payload is the full 1,755-board table (~9 MB) so the writes really overlap, and the race is
    /// run several times because a single round can serialise by luck.
    #[test]
    fn concurrent_writes_to_one_path_leave_one_whole_table() {
        let dir = std::env::temp_dir().join(format!("sv10-tables-race-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let keys = canonical_boards(3);
        assert_eq!(keys.len(), 1_755);
        let table = |v: f32| Table::new(keys.clone(), vec![v; keys.len() * NUM_COMBOS]);
        let (a, b) = (table(1.0), table(2.0));
        let path = table_path(&dir, 3);
        for round in 0..6 {
            let gate = std::sync::Arc::new(std::sync::Barrier::new(2));
            let results: Vec<std::io::Result<()>> = std::thread::scope(|scope| {
                let handles: Vec<_> = [&a, &b]
                    .into_iter()
                    .map(|t| {
                        let (gate, path) = (gate.clone(), path.clone());
                        scope.spawn(move || {
                            gate.wait();
                            t.write(&path, 3)
                        })
                    })
                    .collect();
                handles.into_iter().map(|h| h.join().expect("no panic")).collect()
            });
            // A writer whose staging file was renamed out from under it fails outright — the other
            // half of the same defect: one build racing another must not make either of them fail.
            for (i, r) in results.iter().enumerate() {
                r.as_ref().unwrap_or_else(|e| panic!("round {round}: writer {i} failed: {e}"));
            }
            let read = Table::read(&path, 3).unwrap_or_else(|e| panic!("round {round}: two writers left an unreadable table: {e}"));
            let row = read.lookup(&cards_of(keys[0])).expect("the file's own board is in it");
            assert!(
                row.iter().all(|v| *v == 1.0) || row.iter().all(|v| *v == 2.0),
                "round {round}: the file is a mixture of the two writers"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
