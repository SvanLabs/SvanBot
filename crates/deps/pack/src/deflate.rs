//! Raw DEFLATE compression, levels 0-9 (0258).

use super::tables::*;

/// LSB-first bit writer.
struct BitOut {
    out: Vec<u8>,
    buf: u64,
    count: u32,
}

impl BitOut {
    #[inline]
    fn put(&mut self, value: u32, n: u32) {
        debug_assert!(n <= 32 && (n == 32 || value >> n == 0));
        self.buf |= (value as u64) << self.count;
        self.count += n;
        if self.count >= 32 {
            self.out.extend_from_slice(&(self.buf as u32).to_le_bytes());
            self.buf >>= 32;
            self.count -= 32;
        }
    }

    /// Pad with zero bits to a byte boundary and flush.
    fn align(&mut self) {
        while self.count > 0 {
            self.out.push(self.buf as u8);
            self.buf >>= 8;
            self.count = self.count.saturating_sub(8);
        }
        self.buf = 0;
    }
}

/// Per-level match search settings (zlib's configuration table).
#[derive(Clone, Copy)]
struct Config {
    /// Stop lengthening a lazy match beyond this.
    lazy: usize,
    /// Accept a match this long at once.
    nice: usize,
    /// Hash-chain positions examined per search.
    chain: usize,
    /// Quarter the chain when the deferred match is already this long.
    good: usize,
}

fn config(level: u8) -> Config {
    let (good, lazy, nice, chain) = match level {
        1 => (4, 0, 8, 4),
        2 => (4, 0, 16, 8),
        3 => (4, 0, 32, 32),
        4 => (4, 4, 16, 16),
        5 => (8, 16, 32, 32),
        6 => (8, 16, 128, 128),
        7 => (8, 32, 128, 256),
        8 => (32, 128, 258, 1024),
        _ => (32, 258, 258, 4096),
    };
    Config { lazy, nice, chain, good }
}

pub(super) const WINDOW: usize = 1 << 15;
const HASH_BITS: u32 = 15;
const NONE: u32 = u32::MAX;
const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = 258;
/// Symbols per block before it is emitted (zlib's default literal buffer).
pub(super) const BLOCK_SYMBOLS: usize = 16_384;
/// A 3-byte match farther than this costs more bits than three literals (zlib's TOO_FAR).
const TOO_FAR: usize = 4096;
/// Token tag for a match: `MATCH | length << 16 | distance`.
const MATCH: u32 = 1 << 31;

#[inline]
fn hash3(data: &[u8], i: usize) -> usize {
    let v = (data[i] as u32) << 16 | (data[i + 1] as u32) << 8 | data[i + 2] as u32;
    (v.wrapping_mul(0x9E37_79B1) >> (32 - HASH_BITS)) as usize
}

/// Length of the common prefix of `data[a..]` and `data[b..]`, up to `max`, 8 bytes at a time.
#[inline]
fn common(data: &[u8], a: usize, b: usize, max: usize) -> usize {
    let mut n = 0;
    while n + 8 <= max {
        let x = u64::from_le_bytes(data[a + n..a + n + 8].try_into().unwrap());
        let y = u64::from_le_bytes(data[b + n..b + n + 8].try_into().unwrap());
        let diff = x ^ y;
        if diff != 0 {
            return n + (diff.trailing_zeros() / 8) as usize;
        }
        n += 8;
    }
    while n < max && data[a + n] == data[b + n] {
        n += 1;
    }
    n
}

struct Matcher<'a> {
    data: &'a [u8],
    head: Vec<u32>,
    prev: Vec<u32>,
}

impl<'a> Matcher<'a> {
    fn new(data: &'a [u8]) -> Self {
        Matcher { data, head: vec![NONE; 1 << HASH_BITS], prev: vec![NONE; WINDOW] }
    }

    #[inline]
    fn insert(&mut self, i: usize) {
        if i + MIN_MATCH <= self.data.len() {
            let h = hash3(self.data, i);
            self.prev[i & (WINDOW - 1)] = self.head[h];
            self.head[h] = i as u32;
        }
    }

    /// Longest earlier match for position `i` (before `i` is inserted): (length, distance), or
    /// length 0.
    fn longest(&self, i: usize, cfg: &Config, prev_len: usize) -> (usize, usize) {
        let data = self.data;
        let max = (data.len() - i).min(MAX_MATCH);
        if max < MIN_MATCH {
            return (0, 0);
        }
        let mut chain = if prev_len >= cfg.good { cfg.chain / 4 } else { cfg.chain }.max(1);
        let nice = cfg.nice.min(max);
        let (mut best, mut best_dist) = (prev_len.max(MIN_MATCH - 1), 0);
        let mut cand = self.head[hash3(data, i)];
        while cand != NONE {
            let c = cand as usize;
            if c >= i || i - c > WINDOW {
                break;
            }
            if best < max && data[c + best] == data[i + best] {
                let len = common(data, c, i, max);
                if len > best {
                    best = len;
                    best_dist = i - c;
                    if len >= nice {
                        break;
                    }
                }
            }
            chain -= 1;
            if chain == 0 {
                break;
            }
            let next = self.prev[c & (WINDOW - 1)];
            if next == NONE || next as usize >= c {
                break;
            }
            cand = next;
        }
        if best_dist == 0 || (best == MIN_MATCH && best_dist > TOO_FAR) { (0, 0) } else { (best, best_dist) }
    }
}

/// Compress `data` into a raw DEFLATE stream. `level` 0 stores, 1–3 match greedily, 4–9 match
/// lazily with longer searches (above 9 means 9).
pub fn deflate(data: &[u8], level: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 3 + 64);
    deflate_into(data, level, &mut out);
    out
}

/// [`deflate`] appending to `out`.
pub fn deflate_into(data: &[u8], level: u8, out: &mut Vec<u8>) {
    deflate_dict_into(data, &[], level, out)
}

/// Compress `data` against a preset dictionary (its last 32 KB are the window the stream starts
/// with); decompress with [`inflate_dict`] and the same dictionary.
pub fn deflate_dict(data: &[u8], dict: &[u8], level: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 3 + 64);
    deflate_dict_into(data, dict, level, &mut out);
    out
}

/// [`deflate_dict`] appending to `out`.
pub fn deflate_dict_into(data: &[u8], dict: &[u8], level: u8, out: &mut Vec<u8>) {
    if dict.is_empty() {
        deflate_core(data, None, level, out);
    } else {
        deflate_core(data, Some(&Dictionary::new(dict)), level, out);
    }
}

/// A preset dictionary prepared once: its last 32 KB and the hash chains over them, so compressing
/// many small rows against it costs one table copy each instead of re-hashing the dictionary (0229).
#[derive(Clone)]
pub struct Dictionary {
    bytes: Vec<u8>,
    head: Vec<u32>,
    prev: Vec<u32>,
    /// Unique per prepared dictionary (the decode window cache's key; never reused).
    serial: u64,
}

static DICTIONARY_SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Dictionary {
    /// Prepare `bytes` (only the last 32 KB are used, as in the format).
    pub fn new(bytes: &[u8]) -> Dictionary {
        let bytes = bytes[bytes.len().saturating_sub(WINDOW)..].to_vec();
        let mut m = Matcher::new(&bytes);
        // Positions whose 3-byte hash would read past the dictionary are inserted per row instead.
        for k in 0..bytes.len().saturating_sub(MIN_MATCH - 1) {
            m.insert(k);
        }
        let (head, prev) = (m.head, m.prev);
        let serial = DICTIONARY_SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Dictionary { bytes, head, prev, serial }
    }

    /// The dictionary's bytes (what [`inflate_dict`] needs).
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Unique id for the decode window cache (what `inflate_prepared` keys on).
    pub(super) fn serial(&self) -> u64 {
        self.serial
    }
}

/// [`deflate_dict_into`] with a prepared dictionary.
pub fn deflate_prepared_into(data: &[u8], dict: &Dictionary, level: u8, out: &mut Vec<u8>) {
    deflate_core(data, Some(dict), level, out);
}

pub(super) fn deflate_core(data: &[u8], dict: Option<&Dictionary>, level: u8, out: &mut Vec<u8>) {
    let mut w = BitOut { out: std::mem::take(out), buf: 0, count: 0 };
    if level == 0 || data.is_empty() {
        write_stored(&mut w, data, true);
        w.align();
        *out = w.out;
        return;
    }
    let dlen = dict.map_or(0, |d| d.bytes.len());
    let joined: std::borrow::Cow<'_, [u8]> = match dict {
        None => data.into(),
        Some(d) => [&d.bytes[..], data].concat().into(),
    };
    let data: &[u8] = &joined;
    let cfg = config(level.min(9));
    let lazy = level >= 4;
    let mut m = match dict {
        None => Matcher::new(data),
        Some(d) => {
            let mut m = Matcher { data, head: d.head.clone(), prev: d.prev.clone() };
            // The last dictionary positions hash across into the row.
            for k in dlen.saturating_sub(MIN_MATCH - 1)..dlen {
                m.insert(k);
            }
            m
        }
    };
    let dict = &data[..dlen];
    let mut tokens: Vec<u32> = Vec::with_capacity(BLOCK_SYMBOLS + 2);
    let mut block_start = dict.len();
    let mut i = dict.len();
    // A match found at `pending.0` and held back one position in case a longer one starts there.
    let mut pending: Option<(usize, usize, usize)> = None;
    while i < data.len() {
        let floor = pending.map_or(0, |p| p.1);
        let (len, dist) = if pending.is_none() || floor < cfg.lazy { m.longest(i, &cfg, floor) } else { (0, 0) };
        m.insert(i);
        match pending {
            Some((p, plen, pdist)) => {
                if len > plen {
                    tokens.push(data[p] as u32);
                    pending = Some((i, len, dist));
                    i += 1;
                } else {
                    tokens.push(MATCH | (plen as u32) << 16 | pdist as u32);
                    for k in i + 1..p + plen {
                        m.insert(k);
                    }
                    i = p + plen;
                    pending = None;
                }
            }
            None if len >= MIN_MATCH => {
                if lazy && len < cfg.nice {
                    pending = Some((i, len, dist));
                    i += 1;
                } else {
                    tokens.push(MATCH | (len as u32) << 16 | dist as u32);
                    for k in i + 1..i + len {
                        m.insert(k);
                    }
                    i += len;
                }
            }
            None => {
                tokens.push(data[i] as u32);
                i += 1;
            }
        }
        if pending.is_none() && tokens.len() >= BLOCK_SYMBOLS && i < data.len() {
            write_block(&mut w, &tokens, &data[block_start..i], false);
            tokens.clear();
            block_start = i;
        }
    }
    if let Some((_, plen, pdist)) = pending {
        tokens.push(MATCH | (plen as u32) << 16 | pdist as u32);
    }
    write_block(&mut w, &tokens, &data[block_start..], true);
    w.align();
    *out = w.out;
}

fn write_stored(w: &mut BitOut, raw: &[u8], last: bool) {
    let mut chunks = raw.chunks(0xFFFF).peekable();
    if chunks.peek().is_none() {
        w.put(last as u32, 1);
        w.put(0, 2);
        w.align();
        w.out.extend_from_slice(&[0, 0, 0xFF, 0xFF]);
        return;
    }
    while let Some(c) = chunks.next() {
        let final_chunk = last && chunks.peek().is_none();
        w.put(final_chunk as u32, 1);
        w.put(0, 2);
        w.align();
        let n = c.len() as u16;
        w.out.extend_from_slice(&n.to_le_bytes());
        w.out.extend_from_slice(&(!n).to_le_bytes());
        w.out.extend_from_slice(c);
    }
}

/// Emit one block as stored, fixed or dynamic Huffman, whichever is smallest.
fn write_block(w: &mut BitOut, tokens: &[u32], raw: &[u8], last: bool) {
    let mut lf = [0u32; LITLEN_CODES];
    let mut df = [0u32; DIST_CODES];
    for &t in tokens {
        if t & MATCH != 0 {
            lf[257 + len_code(((t >> 16) & 0x1FF) as usize)] += 1;
            df[dist_code((t & 0xFFFF) as usize)] += 1;
        } else {
            lf[t as usize] += 1;
        }
    }
    lf[END_OF_BLOCK] = 1;
    let extra: u64 = tokens
        .iter()
        .filter(|&&t| t & MATCH != 0)
        .map(|&t| LEN_EXTRA[len_code(((t >> 16) & 0x1FF) as usize)] as u64 + DIST_EXTRA[dist_code((t & 0xFFFF) as usize)] as u64)
        .sum();

    let fixed_l = fixed_litlen_lengths();
    let fixed_bits = 3 + extra + cost(&lf, &fixed_l[..LITLEN_CODES]) + cost(&df, &[5u8; DIST_CODES]);

    let mut ll = code_lengths(&lf, 15);
    let mut dl = code_lengths(&df, 15);
    // At least one distance code (a lone code of length 1 is the permitted incomplete code).
    if dl.iter().all(|&l| l == 0) {
        dl[0] = 1;
    }
    let nlen = (257..=LITLEN_CODES).rev().find(|&n| ll[n - 1] != 0).unwrap_or(257).max(257);
    let ndist = (1..=DIST_CODES).rev().find(|&n| dl[n - 1] != 0).unwrap_or(1);
    let mut all = Vec::with_capacity(nlen + ndist);
    all.extend_from_slice(&ll[..nlen]);
    all.extend_from_slice(&dl[..ndist]);
    let rle = rle_lengths(&all);
    let mut cf = [0u32; 19];
    for &(sym, _) in &rle {
        cf[sym as usize] += 1;
    }
    let cl = code_lengths(&cf, 7);
    let ncode = (4..=19).rev().find(|&n| cl[CL_ORDER[n - 1]] != 0).unwrap_or(4);
    let rle_extra: u64 = rle.iter().map(|&(s, _)| [2u64, 3, 7][(s as usize).saturating_sub(16).min(2)] * (s >= 16) as u64).sum();
    let dyn_bits = 3 + 5 + 5 + 4 + 3 * ncode as u64 + cost(&cf, &cl) + rle_extra + extra + cost(&lf, &ll) + cost(&df, &dl);

    let stored_bits = {
        let blocks = raw.len().div_ceil(0xFFFF).max(1) as u64;
        // Header and padding (at most 3 + 7 bits) plus LEN/NLEN per block, then the bytes.
        blocks * (10 + 32) + raw.len() as u64 * 8
    };

    if stored_bits <= fixed_bits.min(dyn_bits) {
        write_stored(w, raw, last);
        return;
    }
    w.put(last as u32, 1);
    if fixed_bits <= dyn_bits {
        w.put(1, 2);
        let lc = canonical(&fixed_l);
        let dc = canonical(&[5u8; 32]);
        write_tokens(w, tokens, &fixed_l, &lc, &[5u8; 32], &dc);
        return;
    }
    w.put(2, 2);
    w.put((nlen - 257) as u32, 5);
    w.put((ndist - 1) as u32, 5);
    w.put((ncode - 4) as u32, 4);
    for &i in &CL_ORDER[..ncode] {
        w.put(cl[i] as u32, 3);
    }
    let cc = canonical(&cl);
    for &(sym, extra) in &rle {
        w.put(cc[sym as usize], cl[sym as usize] as u32);
        match sym {
            16 => w.put(extra as u32, 2),
            17 => w.put(extra as u32, 3),
            18 => w.put(extra as u32, 7),
            _ => {}
        }
    }
    ll.truncate(LITLEN_CODES);
    dl.truncate(DIST_CODES);
    let lc = canonical(&ll);
    let dc = canonical(&dl);
    write_tokens(w, tokens, &ll, &lc, &dl, &dc);
}

fn write_tokens(w: &mut BitOut, tokens: &[u32], ll: &[u8], lc: &[u32], dl: &[u8], dc: &[u32]) {
    for &t in tokens {
        if t & MATCH != 0 {
            let len = ((t >> 16) & 0x1FF) as usize;
            let dist = (t & 0xFFFF) as usize;
            let s = len_code(len);
            w.put(lc[257 + s], ll[257 + s] as u32);
            if LEN_EXTRA[s] > 0 {
                w.put((len - LEN_BASE[s] as usize) as u32, LEN_EXTRA[s] as u32);
            }
            let ds = dist_code(dist);
            w.put(dc[ds], dl[ds] as u32);
            if DIST_EXTRA[ds] > 0 {
                w.put((dist - DIST_BASE[ds] as usize) as u32, DIST_EXTRA[ds] as u32);
            }
        } else {
            w.put(lc[t as usize], ll[t as usize] as u32);
        }
    }
    w.put(lc[END_OF_BLOCK], ll[END_OF_BLOCK] as u32);
}

fn cost(freq: &[u32], lengths: &[u8]) -> u64 {
    freq.iter().zip(lengths).map(|(&f, &l)| f as u64 * l as u64).sum()
}

/// Canonical codes (RFC 1951 §3.2.2), bit-reversed for LSB-first output.
fn canonical(lengths: &[u8]) -> Vec<u32> {
    let mut count = [0u32; MAX_BITS + 1];
    for &l in lengths {
        count[l as usize] += 1;
    }
    count[0] = 0;
    let mut next = [0u32; MAX_BITS + 2];
    let mut code = 0;
    for len in 1..=MAX_BITS {
        code = (code + count[len - 1]) << 1;
        next[len] = code;
    }
    lengths
        .iter()
        .map(|&l| {
            if l == 0 {
                return 0;
            }
            let c = next[l as usize];
            next[l as usize] += 1;
            c.reverse_bits() >> (32 - l as u32)
        })
        .collect()
}

/// Run-length code the code lengths with symbols 16 (repeat previous 3–6), 17 (zeros 3–10) and
/// 18 (zeros 11–138): (symbol, extra-bit value).
fn rle_lengths(lengths: &[u8]) -> Vec<(u8, u8)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lengths.len() {
        let v = lengths[i];
        let mut run = 1;
        while i + run < lengths.len() && lengths[i + run] == v {
            run += 1;
        }
        i += run;
        if v == 0 {
            while run >= 11 {
                let n = run.min(138);
                out.push((18, (n - 11) as u8));
                run -= n;
            }
            if run >= 3 {
                out.push((17, (run - 3) as u8));
                run = 0;
            }
            out.extend(std::iter::repeat_n((0, 0), run));
        } else {
            out.push((v, 0));
            run -= 1;
            while run >= 3 {
                let n = run.min(6);
                out.push((16, (n - 3) as u8));
                run -= n;
            }
            out.extend(std::iter::repeat_n((v, 0), run));
        }
    }
    out
}

/// Length-limited Huffman code lengths for `freq` (zero frequency → length 0): minimum-redundancy
/// lengths by the in-place method of Moffat and Katajainen, then lengths above `limit` folded back
/// with the Kraft sum kept exact (as miniz does).
pub(super) fn code_lengths(freq: &[u32], limit: usize) -> Vec<u8> {
    let mut syms: Vec<(u32, usize)> = freq.iter().enumerate().filter(|&(_, &f)| f > 0).map(|(s, &f)| (f, s)).collect();
    let mut lengths = vec![0u8; freq.len()];
    match syms.len() {
        0 => return lengths,
        1 => {
            lengths[syms[0].1] = 1;
            return lengths;
        }
        _ => {}
    }
    syms.sort_unstable();
    let n = syms.len();
    let mut a: Vec<u32> = syms.iter().map(|s| s.0).collect();
    // Phase 1: parent pointers and internal node weights.
    a[0] += a[1];
    let (mut root, mut leaf) = (0usize, 2usize);
    for next in 1..n - 1 {
        if leaf >= n || a[root] < a[leaf] {
            a[next] = a[root];
            a[root] = next as u32;
            root += 1;
        } else {
            a[next] = a[leaf];
            leaf += 1;
        }
        if leaf >= n || (root < next && a[root] < a[leaf]) {
            a[next] += a[root];
            a[root] = next as u32;
            root += 1;
        } else {
            a[next] += a[leaf];
            leaf += 1;
        }
    }
    // Phase 2: internal node depths.
    a[n - 2] = 0;
    for next in (0..n.saturating_sub(2)).rev() {
        a[next] = a[a[next] as usize] + 1;
    }
    // Phase 3: leaf depths.
    let (mut avbl, mut used, mut dpth) = (1i64, 0i64, 0u32);
    let (mut root, mut next) = (n as i64 - 2, n as i64 - 1);
    while avbl > 0 {
        while root >= 0 && a[root as usize] == dpth {
            used += 1;
            root -= 1;
        }
        while avbl > used {
            a[next as usize] = dpth;
            next -= 1;
            avbl -= 1;
        }
        avbl = 2 * used;
        dpth += 1;
        used = 0;
    }
    // Count codes per length, fold overlong ones into `limit` and restore the Kraft sum.
    let mut num = [0u32; 33];
    for &d in &a {
        num[(d as usize).min(32)] += 1;
    }
    for i in limit + 1..=32 {
        num[limit] += num[i];
        num[i] = 0;
    }
    let mut total: u64 = (1..=limit).map(|i| (num[i] as u64) << (limit - i)).sum();
    while total > 1u64 << limit {
        num[limit] -= 1;
        for i in (1..limit).rev() {
            if num[i] != 0 {
                num[i] -= 1;
                num[i + 1] += 2;
                break;
            }
        }
        total -= 1;
    }
    // Most frequent symbols get the shortest codes.
    let mut j = n;
    for (len, &count) in num.iter().enumerate().take(limit + 1).skip(1) {
        for _ in 0..count {
            j -= 1;
            lengths[syms[j].1] = len as u8;
        }
    }
    lengths
}
