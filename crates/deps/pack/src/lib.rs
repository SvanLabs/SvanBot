//! Raw DEFLATE (RFC 1951) compression and decompression, CRC-32, and a small checked frame for
//! stored blobs, with no dependencies.
//!
//! Part of the svanbot10 workspace (0229): the cold JSON that fills the databases (server hand
//! exports, decision details, replay records) is stored compressed so the SSD keeps less and
//! writes less. The decoder is checked against streams produced by zlib (`tests/zlib.rs`), the
//! encoder by round trips through the decoder and by zlib decoding its output
//! (`scripts/tests/test_pack.py`).
//!
//! - [`deflate`] / [`inflate`]: raw DEFLATE streams (no zlib or gzip header), levels 0–9 like
//!   zlib's (0 stores, 1–3 match greedily, 4–9 match lazily with longer hash chains).
//! - [`pack`] / [`unpack`]: a frame for stored blobs — magic `SVZ`, a preset-dictionary id, the uncompressed length and
//!   its CRC-32, then the stream. [`unpack`] refuses a frame whose length exceeds the caller's
//!   limit before inflating anything, so a corrupt or hostile blob cannot exhaust memory.
//! - [`crc32`]: the CRC-32 of zlib, gzip and PNG (reflected polynomial `0xEDB88320`).

#![warn(missing_docs)]

use std::fmt;

/// Why a stream or frame could not be decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The input ended inside a block.
    Truncated,
    /// Block type 3 (reserved).
    BadBlockType,
    /// A stored block's length and its complement disagree.
    BadStoredLength,
    /// A Huffman code table is over-subscribed, incomplete where that is not allowed, or names a
    /// symbol the format reserves.
    BadCode,
    /// A back-reference points before the start of the output.
    BadDistance,
    /// The output would exceed the caller's limit (the limit is carried).
    TooLarge(usize),
    /// The bytes are not a frame (wrong magic or too short).
    NotAFrame,
    /// The frame's length or CRC-32 does not match its content.
    Corrupt,
    /// The frame was compressed against a preset dictionary the caller does not know (id carried).
    MissingDictionary(u8),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Truncated => write!(f, "deflate stream truncated"),
            Error::BadBlockType => write!(f, "deflate block type 3 is reserved"),
            Error::BadStoredLength => write!(f, "stored block length check failed"),
            Error::BadCode => write!(f, "invalid Huffman code in deflate stream"),
            Error::BadDistance => write!(f, "deflate distance points before the output"),
            Error::TooLarge(limit) => write!(f, "decompressed size exceeds the {limit}-byte limit"),
            Error::NotAFrame => write!(f, "not an SVZ frame"),
            Error::Corrupt => write!(f, "SVZ frame length or CRC-32 mismatch"),
            Error::MissingDictionary(id) => write!(f, "SVZ frame needs preset dictionary {id}"),
        }
    }
}

impl std::error::Error for Error {}

// One module per algorithm (0258); the names below are re-exported so every caller keeps its
// path (`sv10_pack::pack`, `sv10_pack::Dictionary` and friends are unchanged).
mod crc;
mod deflate;
mod frame;
mod inflate;
mod tables;

pub use crc::{crc32, crc32_update};
pub use deflate::{Dictionary, deflate, deflate_dict, deflate_dict_into, deflate_into, deflate_prepared_into};
pub use frame::{
    HEADER, LEVEL, MAGIC, dictionary_id, is_packed, pack, pack_prepared, pack_with, text, text_prepared, text_with, unpack,
    unpack_prepared, unpack_with,
};
pub use inflate::{inflate, inflate_dict, inflate_into, inflate_prepared};

#[cfg(test)]
mod tests;
