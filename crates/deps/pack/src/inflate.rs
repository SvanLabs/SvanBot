//! Raw DEFLATE decompression (0258).

use super::Error;
use super::deflate::{Dictionary, WINDOW};
use super::tables::*;

/// LSB-first bit reader with a 64-bit buffer.
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u64,
    count: u32,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Bits { data, pos: 0, buf: 0, count: 0 }
    }

    #[inline]
    fn refill(&mut self) {
        if self.count <= 32 && self.pos + 4 <= self.data.len() {
            let w = u32::from_le_bytes([self.data[self.pos], self.data[self.pos + 1], self.data[self.pos + 2], self.data[self.pos + 3]]);
            self.buf |= (w as u64) << self.count;
            self.pos += 4;
            self.count += 32;
        }
        while self.count <= 56 && self.pos < self.data.len() {
            self.buf |= (self.data[self.pos] as u64) << self.count;
            self.pos += 1;
            self.count += 8;
        }
    }

    #[inline]
    fn bits(&mut self, n: u32) -> Result<u32, Error> {
        if n == 0 {
            return Ok(0);
        }
        if self.count < n {
            self.refill();
            if self.count < n {
                return Err(Error::Truncated);
            }
        }
        let v = (self.buf & ((1u64 << n) - 1)) as u32;
        self.buf >>= n;
        self.count -= n;
        Ok(v)
    }

    /// Drop the bits up to the next byte boundary (stored blocks).
    fn align(&mut self) {
        let drop = self.count % 8;
        self.buf >>= drop;
        self.count -= drop;
    }

    /// Copy `n` whole bytes to `out` after [`align`](Bits::align): buffered bytes first.
    fn copy_bytes(&mut self, mut n: usize, out: &mut Vec<u8>) -> Result<(), Error> {
        while n > 0 && self.count >= 8 {
            out.push((self.buf & 0xFF) as u8);
            self.buf >>= 8;
            self.count -= 8;
            n -= 1;
        }
        if n > 0 {
            let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len()).ok_or(Error::Truncated)?;
            out.extend_from_slice(&self.data[self.pos..end]);
            self.pos = end;
        }
        Ok(())
    }
}

/// Bits resolved by one lookup in [`Huffman::fast`]; longer codes finish canonically.
const FAST_BITS: u32 = 10;

/// A canonical Huffman decoder: a direct table for codes of up to [`FAST_BITS`] bits and the
/// per-length counts and sorted symbols for longer ones.
struct Huffman {
    /// Indexed by the next `FAST_BITS` stream bits: `symbol | length << 9`, or 0 for a longer code.
    fast: Vec<u16>,
    count: [u16; MAX_BITS + 1],
    symbols: Vec<u16>,
}

impl Huffman {
    /// Build from code lengths. `allow_single` accepts the one incomplete code RFC 1951 permits:
    /// a lone code of length 1 (a block with one distance, or only end-of-block).
    fn new(lengths: &[u8], allow_incomplete: bool) -> Result<Huffman, Error> {
        let mut count = [0u16; MAX_BITS + 1];
        for &l in lengths {
            count[l as usize] += 1;
        }
        let mut left: i32 = 1;
        for &c in &count[1..] {
            left = left * 2 - c as i32;
            if left < 0 {
                return Err(Error::BadCode);
            }
        }
        let used = lengths.len() - count[0] as usize;
        if left > 0 && !(allow_incomplete && used <= 1) {
            return Err(Error::BadCode);
        }
        let mut offs = [0u16; MAX_BITS + 2];
        for len in 1..=MAX_BITS {
            offs[len + 1] = offs[len] + count[len];
        }
        let mut symbols = vec![0u16; used];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[offs[l as usize] as usize] = sym as u16;
                offs[l as usize] += 1;
            }
        }
        // Canonical codes, reversed for LSB-first reading, spread over the fast table.
        let mut fast = vec![0u16; 1 << FAST_BITS];
        let mut code = 0u32;
        let mut next = [0u32; MAX_BITS + 2];
        for len in 1..=MAX_BITS {
            code = (code + count[len - 1] as u32) << 1;
            next[len] = code;
        }
        for (sym, &l) in lengths.iter().enumerate() {
            let l = l as u32;
            if l == 0 {
                continue;
            }
            let c = next[l as usize];
            next[l as usize] += 1;
            if l <= FAST_BITS {
                let rev = c.reverse_bits() >> (32 - l);
                let entry = sym as u16 | (l as u16) << 9;
                let mut i = rev as usize;
                while i < fast.len() {
                    fast[i] = entry;
                    i += 1 << l;
                }
            }
        }
        Ok(Huffman { fast, count, symbols })
    }

    #[inline]
    fn decode(&self, bits: &mut Bits<'_>) -> Result<usize, Error> {
        if bits.count < MAX_BITS as u32 {
            bits.refill();
        }
        let entry = self.fast[(bits.buf & ((1 << FAST_BITS) - 1)) as usize];
        if entry != 0 {
            let len = (entry >> 9) as u32;
            if len > bits.count {
                return Err(Error::Truncated);
            }
            bits.buf >>= len;
            bits.count -= len;
            return Ok((entry & 0x1FF) as usize);
        }
        // Canonical decoding one bit at a time (codes longer than FAST_BITS, or an unused prefix).
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..=MAX_BITS {
            if len as u32 > bits.count {
                return Err(Error::Truncated);
            }
            code |= ((bits.buf >> (len - 1)) & 1) as i32;
            let count = self.count[len] as i32;
            if code - count < first {
                bits.buf >>= len;
                bits.count -= len as u32;
                return Ok(self.symbols[(index + (code - first)) as usize] as usize);
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err(Error::BadCode)
    }
}

/// Decompress a raw DEFLATE stream, refusing to produce more than `limit` bytes. Bytes after the
/// final block are ignored.
pub fn inflate(input: &[u8], limit: usize) -> Result<Vec<u8>, Error> {
    let mut out = Vec::with_capacity(input.len().saturating_mul(4).min(limit));
    inflate_into(input, limit, &mut out)?;
    Ok(out)
}

/// [`inflate`] appending to `out`; `limit` bounds what this call appends. Back-references reach
/// only bytes this call produced.
pub fn inflate_into(input: &[u8], limit: usize, out: &mut Vec<u8>) -> Result<(), Error> {
    let start = out.len();
    inflate_window(input, limit, out, start)
}

/// Decompress a raw DEFLATE stream that was compressed against the preset dictionary `dict`
/// (zlib's `inflateSetDictionary` on a raw stream).
pub fn inflate_dict(input: &[u8], dict: &[u8], limit: usize) -> Result<Vec<u8>, Error> {
    let dict = &dict[dict.len().saturating_sub(WINDOW)..];
    let mut buf = Vec::with_capacity(dict.len() + input.len().saturating_mul(4).min(limit));
    buf.extend_from_slice(dict);
    inflate_window(input, limit, &mut buf, 0)?;
    // An exact-size copy: draining the dictionary off the front would keep its 32 KB of capacity
    // in every value returned (68 MB of stored details held 2.3 GB that way, 0229).
    Ok(buf[dict.len()..].to_vec())
}

thread_local! {
    /// Per-thread decode window: the prepared dictionary last used on this thread (by serial) followed
    /// by the stream being decoded, so decoding many small values against one dictionary copies the
    /// 32 KB dictionary once per thread instead of once per value.
    static DECODE_WINDOW: std::cell::RefCell<(u64, Vec<u8>)> = const { std::cell::RefCell::new((0, Vec::new())) };
}

/// Decoded bytes a thread's window keeps capacity for between values (larger values still decode;
/// the window shrinks back afterwards).
const KEEP_WINDOW: usize = 1 << 20;

/// [`inflate_dict`] with a prepared dictionary: the same output, without re-copying the dictionary
/// for every value decoded on the same thread.
pub fn inflate_prepared(input: &[u8], dict: &Dictionary, limit: usize) -> Result<Vec<u8>, Error> {
    DECODE_WINDOW.with(|cell| {
        let mut guard = cell.borrow_mut();
        let (serial, buf) = &mut *guard;
        let base = dict.bytes().len();
        if *serial != dict.serial() || buf.len() < base {
            buf.clear();
            buf.extend_from_slice(dict.bytes());
            *serial = dict.serial();
        }
        buf.truncate(base);
        let result = inflate_window(input, limit, buf, 0).map(|()| buf[base..].to_vec());
        buf.truncate(base);
        if buf.capacity() > base + KEEP_WINDOW {
            buf.shrink_to(base + KEEP_WINDOW);
        }
        result
    })
}

/// Inflate appending to `out`; back-references may reach back to `out[window..]`.
fn inflate_window(input: &[u8], limit: usize, out: &mut Vec<u8>, window: usize) -> Result<(), Error> {
    let start = out.len();
    let max = start.saturating_add(limit);
    let mut bits = Bits::new(input);
    loop {
        let last = bits.bits(1)? == 1;
        match bits.bits(2)? {
            0 => {
                bits.align();
                let len = bits.bits(16)? as usize;
                let nlen = bits.bits(16)? as usize;
                if len != !nlen & 0xFFFF {
                    return Err(Error::BadStoredLength);
                }
                if out.len() + len > max {
                    return Err(Error::TooLarge(limit));
                }
                bits.copy_bytes(len, out)?;
            }
            1 => {
                let (lit, dist) = fixed_decoders();
                codes(&mut bits, lit, dist, out, window, max, limit)?;
            }
            2 => {
                let (lit, dist) = dynamic_decoders(&mut bits)?;
                codes(&mut bits, &lit, &dist, out, window, max, limit)?;
            }
            _ => return Err(Error::BadBlockType),
        }
        if last {
            return Ok(());
        }
    }
}

fn fixed_decoders() -> (&'static Huffman, &'static Huffman) {
    static FIXED: std::sync::OnceLock<(Huffman, Huffman)> = std::sync::OnceLock::new();
    let (l, d) = FIXED.get_or_init(|| {
        let lit = Huffman::new(&fixed_litlen_lengths(), false).expect("fixed literal code");
        // 30 used distance codes of length 5 (of 32 slots): incomplete by design.
        let mut dl = [5u8; 32];
        dl[30] = 0;
        dl[31] = 0;
        let dist = Huffman::new_fixed_dist(&dl);
        (lit, dist)
    });
    (l, d)
}

impl Huffman {
    /// The fixed distance code (incomplete by the format's definition, so built without the check).
    fn new_fixed_dist(lengths: &[u8]) -> Huffman {
        let mut all = [5u8; 32];
        all[..lengths.len()].copy_from_slice(lengths);
        all[30] = 5;
        all[31] = 5;
        // Symbols 30 and 31 decode but are rejected where distances are used.
        Huffman::new(&all, false).expect("fixed distance code")
    }
}

fn dynamic_decoders(bits: &mut Bits<'_>) -> Result<(Huffman, Huffman), Error> {
    let nlen = bits.bits(5)? as usize + 257;
    let ndist = bits.bits(5)? as usize + 1;
    let ncode = bits.bits(4)? as usize + 4;
    if nlen > LITLEN_CODES || ndist > DIST_CODES {
        return Err(Error::BadCode);
    }
    let mut cl = [0u8; 19];
    for &i in &CL_ORDER[..ncode] {
        cl[i] = bits.bits(3)? as u8;
    }
    let clcode = Huffman::new(&cl, false)?;
    let mut lengths = [0u8; LITLEN_CODES + DIST_CODES];
    let mut i = 0;
    while i < nlen + ndist {
        let sym = clcode.decode(bits)?;
        if sym < 16 {
            lengths[i] = sym as u8;
            i += 1;
            continue;
        }
        let (value, repeat) = match sym {
            16 => {
                if i == 0 {
                    return Err(Error::BadCode);
                }
                (lengths[i - 1], 3 + bits.bits(2)? as usize)
            }
            17 => (0, 3 + bits.bits(3)? as usize),
            _ => (0, 11 + bits.bits(7)? as usize),
        };
        if i + repeat > nlen + ndist {
            return Err(Error::BadCode);
        }
        lengths[i..i + repeat].fill(value);
        i += repeat;
    }
    if lengths[END_OF_BLOCK] == 0 {
        return Err(Error::BadCode);
    }
    let lit = Huffman::new(&lengths[..nlen], true)?;
    let dist = Huffman::new(&lengths[nlen..nlen + ndist], true)?;
    Ok((lit, dist))
}

fn codes(
    bits: &mut Bits<'_>,
    lit: &Huffman,
    dist: &Huffman,
    out: &mut Vec<u8>,
    window: usize,
    max: usize,
    limit: usize,
) -> Result<(), Error> {
    loop {
        let sym = lit.decode(bits)?;
        if sym < 256 {
            if out.len() >= max {
                return Err(Error::TooLarge(limit));
            }
            out.push(sym as u8);
        } else if sym == END_OF_BLOCK {
            return Ok(());
        } else {
            let s = sym - 257;
            if s >= 29 {
                return Err(Error::BadCode);
            }
            let len = LEN_BASE[s] as usize + bits.bits(LEN_EXTRA[s] as u32)? as usize;
            let ds = dist.decode(bits)?;
            if ds >= DIST_CODES {
                return Err(Error::BadCode);
            }
            let d = DIST_BASE[ds] as usize + bits.bits(DIST_EXTRA[ds] as u32)? as usize;
            if d > out.len() - window {
                return Err(Error::BadDistance);
            }
            if out.len() + len > max {
                return Err(Error::TooLarge(limit));
            }
            let from = out.len() - d;
            if d >= len {
                out.extend_from_within(from..from + len);
            } else {
                // Overlapping copy: the pattern of the last `d` bytes repeats.
                for k in 0..len {
                    let b = out[from + k];
                    out.push(b);
                }
            }
        }
    }
}
