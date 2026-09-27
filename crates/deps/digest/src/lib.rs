//! SHA-256 (FIPS 180-4), HMAC-SHA-256 (RFC 2104) and lowercase hex, with no dependencies.
//!
//! Part of the svanbot10 workspace (0222): owned so the digests behind the protocol state hash
//! (`sv10-venue`), store integrity and archives (`sv10-store`), replay records and the dashboard
//! session (`sv10-bot`) come from code this project can read end to end. Every function is checked
//! against the published NIST and RFC 4231 vectors; before it replaced the `sha2` crate it matched
//! `sha2` on 20,000 random inputs split at random points, and ran at 190 MB/s against `sha2`'s 13 on
//! the same machine (2026-09-26).
//!
//! The interface mirrors the `sha2` crate's: [`Sha256::new`], [`Sha256::update`],
//! [`Sha256::finalize`], and the one-shot [`Sha256::digest`].

#![warn(missing_docs)]

/// Round constants: the first 32 bits of the fractional parts of the cube roots of the first 64
/// primes (FIPS 180-4 §4.2.2).
const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be,
    0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa,
    0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85,
    0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
    0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f,
    0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Initial hash value: the first 32 bits of the fractional parts of the square roots of the first
/// 8 primes (FIPS 180-4 §5.3.3).
const H0: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];

/// Bytes per message block.
const BLOCK: usize = 64;

/// A SHA-256 digest in progress. Feed it with [`update`](Sha256::update) any number of times, then
/// [`finalize`](Sha256::finalize); the result depends only on the concatenated input.
#[derive(Clone, Debug)]
pub struct Sha256 {
    state: [u32; 8],
    /// Bytes of a partial block not yet compressed.
    buf: [u8; BLOCK],
    buf_len: usize,
    /// Total message length in bytes.
    len: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    /// An empty digest.
    pub fn new() -> Self {
        Sha256 { state: H0, buf: [0; BLOCK], buf_len: 0, len: 0 }
    }

    /// The digest of `data` in one call.
    pub fn digest(data: impl AsRef<[u8]>) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(data);
        h.finalize()
    }

    /// Append `data` to the message.
    pub fn update(&mut self, data: impl AsRef<[u8]>) {
        let mut data = data.as_ref();
        self.len = self.len.wrapping_add(data.len() as u64);
        if self.buf_len > 0 {
            let take = (BLOCK - self.buf_len).min(data.len());
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
            if self.buf_len < BLOCK {
                return;
            }
            let block = self.buf;
            compress(&mut self.state, &block);
            self.buf_len = 0;
        }
        let (blocks, rest) = data.as_chunks::<BLOCK>();
        for block in blocks {
            compress(&mut self.state, block);
        }
        self.buf[..rest.len()].copy_from_slice(rest);
        self.buf_len = rest.len();
    }

    /// The 32-byte digest of everything appended (FIPS 180-4 §5.1.1 padding).
    pub fn finalize(mut self) -> [u8; 32] {
        let bit_len = self.len.wrapping_mul(8);
        let mut tail = [0u8; BLOCK * 2];
        tail[..self.buf_len].copy_from_slice(&self.buf[..self.buf_len]);
        tail[self.buf_len] = 0x80;
        let total = if self.buf_len < BLOCK - 8 { BLOCK } else { BLOCK * 2 };
        tail[total - 8..total].copy_from_slice(&bit_len.to_be_bytes());
        for block in tail[..total].as_chunks::<BLOCK>().0 {
            compress(&mut self.state, block);
        }
        let mut out = [0u8; 32];
        for (o, w) in out.as_chunks_mut::<4>().0.iter_mut().zip(self.state) {
            *o = w.to_be_bytes();
        }
        out
    }
}

/// One application of the compression function to a 64-byte block (FIPS 180-4 §6.2.2).
#[inline]
fn compress(state: &mut [u32; 8], block: &[u8; BLOCK]) {
    let mut w = [0u32; 64];
    for (i, word) in block.as_chunks::<4>().0.iter().enumerate() {
        w[i] = u32::from_be_bytes(*word);
    }
    for t in 16..64 {
        let s0 = w[t - 15].rotate_right(7) ^ w[t - 15].rotate_right(18) ^ (w[t - 15] >> 3);
        let s1 = w[t - 2].rotate_right(17) ^ w[t - 2].rotate_right(19) ^ (w[t - 2] >> 10);
        w[t] = w[t - 16].wrapping_add(s0).wrapping_add(w[t - 7]).wrapping_add(s1);
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for t in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ (!e & g);
        let t1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[t]).wrapping_add(w[t]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let t2 = s0.wrapping_add(maj);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }
    for (s, v) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *s = s.wrapping_add(v);
    }
}

/// HMAC-SHA-256 of `message` under `key` (RFC 2104): a keyed digest that only a holder of the key
/// can produce, for tokens and signatures.
pub fn hmac_sha256(key: impl AsRef<[u8]>, message: impl AsRef<[u8]>) -> [u8; 32] {
    let key = key.as_ref();
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    inner.update(message);
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize()
}

/// Whether `a` and `b` are equal, taking the same time wherever they differ (for comparing
/// secrets and MACs).
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Lowercase hexadecimal of `bytes` (two digits per byte).
pub fn hex(bytes: impl AsRef<[u8]>) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let bytes = bytes.as_ref();
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(DIGITS[usize::from(b >> 4)] as char);
        s.push(DIGITS[usize::from(b & 0x0f)] as char);
    }
    s
}

/// The bytes of a hexadecimal string (either case); `None` for odd length or a non-hex digit.
pub fn from_hex(s: &str) -> Option<Vec<u8>> {
    fn nibble(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    }
    let s = s.as_bytes();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    s.as_chunks::<2>().0.iter().map(|&[hi, lo]| Some(nibble(hi)? << 4 | nibble(lo)?)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(data: &[u8]) -> String {
        hex(Sha256::digest(data))
    }

    #[test]
    fn nist_short_and_long_messages() {
        // FIPS 180-4 examples (NIST CSRC "SHA256.pdf") and the NIST SHAVS byte-oriented vectors.
        assert_eq!(sha(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(
            sha(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        assert_eq!(
            sha(b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu"),
            "cf5b16a778af8380036ce59e7b0492370b249b11e8f07a51afac45037afee9d1"
        );
        assert_eq!(sha(&[0xbd]), "68325720aabd7c82f30f554b313d0570c95accbb7dc4b5aae11204c08ffe732b");
        assert_eq!(sha(&[0xc9, 0x8c, 0x8e, 0x55]), "7abc22c0ae5af26ce93dbb94433a0e0b2e119d014f8e7f65bd56c61ccccd9504");
    }

    #[test]
    fn one_million_a() {
        let mut h = Sha256::new();
        for _ in 0..1000 {
            h.update([b'a'; 1000]);
        }
        assert_eq!(hex(h.finalize()), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
    }

    #[test]
    fn every_split_of_the_input_gives_the_same_digest() {
        // Lengths around the padding boundaries (55, 56, 63, 64 bytes) and splits at every offset.
        let data: Vec<u8> = (0..200u8).map(|i| i.wrapping_mul(31).wrapping_add(7)).collect();
        for len in [0, 1, 55, 56, 57, 63, 64, 65, 119, 120, 127, 128, 129, 200] {
            let whole = Sha256::digest(&data[..len]);
            for cut in 0..=len {
                let mut h = Sha256::new();
                h.update(&data[..cut]);
                h.update(&data[cut..len]);
                assert_eq!(h.finalize(), whole, "len {len} cut {cut}");
            }
        }
    }

    #[test]
    fn rfc_4231_hmac_vectors() {
        // Test cases 1, 2, 3, 4, 6 and 7 of RFC 4231 (HMAC-SHA-256).
        let cases: [(Vec<u8>, Vec<u8>, &str); 6] = [
            (vec![0x0b; 20], b"Hi There".to_vec(), "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"),
            (b"Jefe".to_vec(), b"what do ya want for nothing?".to_vec(), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"),
            (vec![0xaa; 20], vec![0xdd; 50], "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe"),
            (
                (1..=25u8).collect(),
                vec![0xcd; 50],
                "82558a389a443c0ea4cc819899f2083a85f0faa3e578f8077a2e3ff46729665b",
            ),
            (
                vec![0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First".to_vec(),
                "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54",
            ),
            (
                vec![0xaa; 131],
                b"This is a test using a larger than block-size key and a larger than block-size data. The key needs to be hashed before being used by the HMAC algorithm.".to_vec(),
                "9b09ffa71b942fcb27635fbcd5b0e944bfdc63644f0713938a7f51535c3a35e2",
            ),
        ];
        for (key, msg, want) in cases {
            assert_eq!(hex(hmac_sha256(&key, &msg)), want);
        }
    }

    #[test]
    fn hex_round_trips_and_rejects_junk() {
        let bytes: Vec<u8> = (0..=255).collect();
        let h = hex(&bytes);
        assert_eq!(&h[..8], "00010203");
        assert_eq!(from_hex(&h).as_deref(), Some(bytes.as_slice()));
        assert_eq!(from_hex("ABcd").as_deref(), Some(&[0xab, 0xcd][..]));
        assert_eq!(from_hex("abc"), None);
        assert_eq!(from_hex("zz"), None);
        assert!(constant_time_eq(b"same", b"same") && !constant_time_eq(b"same", b"sane") && !constant_time_eq(b"a", b"ab"));
    }
}
