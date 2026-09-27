//! Checked SVZ frame for stored blobs (0258).

use super::Error;
use super::crc::crc32;
use super::deflate::{Dictionary, deflate_core};
use super::inflate::{inflate_dict, inflate_into, inflate_prepared};

/// First bytes of every frame; the fourth byte is the preset dictionary id (0 = none).
pub const MAGIC: [u8; 3] = *b"SVZ";
/// Frame header: magic, dictionary id, `u64` uncompressed length, `u32` CRC-32 (little-endian).
pub const HEADER: usize = 16;
/// Level [`pack`] compresses at: zlib's default balance of size and speed.
pub const LEVEL: u8 = 6;

/// Whether `bytes` start like a frame.
pub fn is_packed(bytes: &[u8]) -> bool {
    bytes.len() >= HEADER && bytes[..3] == MAGIC
}

/// The preset dictionary id a frame names (0 = none), if `bytes` is a frame.
pub fn dictionary_id(bytes: &[u8]) -> Option<u8> {
    is_packed(bytes).then(|| bytes[3])
}

/// Compress `data` into a checked frame without a dictionary.
pub fn pack(data: &[u8]) -> Vec<u8> {
    pack_with(data, 0, &[])
}

/// Compress `data` into a checked frame against preset dictionary `dict`, recorded as `id`
/// (1–255). A dictionary id must name the same bytes forever: frames carry only the id.
pub fn pack_with(data: &[u8], id: u8, dict: &[u8]) -> Vec<u8> {
    assert!(id != 0 || dict.is_empty(), "dictionary id 0 means no dictionary");
    if dict.is_empty() {
        return frame(data, id, |out| deflate_core(data, None, LEVEL, out));
    }
    let prepared = Dictionary::new(dict);
    frame(data, id, |out| deflate_core(data, Some(&prepared), LEVEL, out))
}

/// [`pack_with`] with a prepared dictionary (many rows against one dictionary).
pub fn pack_prepared(data: &[u8], id: u8, dict: &Dictionary) -> Vec<u8> {
    assert!(id != 0, "dictionary id 0 means no dictionary");
    frame(data, id, |out| deflate_core(data, Some(dict), LEVEL, out))
}

fn frame(data: &[u8], id: u8, body: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER + data.len() / 3 + 64);
    out.extend_from_slice(&MAGIC);
    out.push(id);
    out.extend_from_slice(&(data.len() as u64).to_le_bytes());
    out.extend_from_slice(&crc32(data).to_le_bytes());
    body(&mut out);
    out
}

/// Decompress a frame made without a dictionary; see [`unpack_with`].
pub fn unpack(frame: &[u8], limit: usize) -> Result<Vec<u8>, Error> {
    unpack_with(frame, limit, |_| None)
}

/// Decompress a frame whose stated length is at most `limit` bytes, verifying its length and CRC-32;
/// `dictionary(id)` supplies the preset dictionary a frame names.
pub fn unpack_with<'d>(frame: &[u8], limit: usize, dictionary: impl Fn(u8) -> Option<&'d [u8]>) -> Result<Vec<u8>, Error> {
    let (id, len, crc) = frame_header(frame, limit)?;
    let out = if id == 0 {
        let mut out = Vec::with_capacity(len);
        inflate_into(&frame[HEADER..], len, &mut out)?;
        out
    } else {
        let dict = dictionary(id).ok_or(Error::MissingDictionary(id))?;
        inflate_dict(&frame[HEADER..], dict, len)?
    };
    if out.len() != len || crc32(&out) != crc {
        return Err(Error::Corrupt);
    }
    Ok(out)
}

/// [`unpack_with`] with prepared dictionaries (the fast path for many small values: see
/// [`inflate_prepared`]).
pub fn unpack_prepared<'d>(frame: &[u8], limit: usize, dictionary: impl Fn(u8) -> Option<&'d Dictionary>) -> Result<Vec<u8>, Error> {
    let (id, len, crc) = frame_header(frame, limit)?;
    let out = if id == 0 {
        let mut out = Vec::with_capacity(len);
        inflate_into(&frame[HEADER..], len, &mut out)?;
        out
    } else {
        let dict = dictionary(id).ok_or(Error::MissingDictionary(id))?;
        inflate_prepared(&frame[HEADER..], dict, len)?
    };
    if out.len() != len || crc32(&out) != crc {
        return Err(Error::Corrupt);
    }
    Ok(out)
}

/// [`text_with`] with prepared dictionaries.
pub fn text_prepared<'d>(bytes: &[u8], limit: usize, dictionary: impl Fn(u8) -> Option<&'d Dictionary>) -> Result<String, Error> {
    let raw = if is_packed(bytes) { unpack_prepared(bytes, limit, dictionary)? } else { bytes.to_vec() };
    String::from_utf8(raw).map_err(|_| Error::Corrupt)
}

/// A frame's dictionary id, uncompressed length (checked against `limit`) and CRC-32.
fn frame_header(frame: &[u8], limit: usize) -> Result<(u8, usize, u32), Error> {
    if !is_packed(frame) {
        return Err(Error::NotAFrame);
    }
    let len = u64::from_le_bytes(frame[4..12].try_into().map_err(|_| Error::NotAFrame)?);
    let crc = u32::from_le_bytes(frame[12..16].try_into().map_err(|_| Error::NotAFrame)?);
    let len = usize::try_from(len).map_err(|_| Error::TooLarge(limit))?;
    if len > limit {
        return Err(Error::TooLarge(limit));
    }
    Ok((frame[3], len, crc))
}

/// The text of a stored value that is either a frame or plain UTF-8 (rows written before
/// compression, or by older builds), limited to `limit` bytes.
pub fn text_with<'d>(bytes: &[u8], limit: usize, dictionary: impl Fn(u8) -> Option<&'d [u8]>) -> Result<String, Error> {
    let raw = if is_packed(bytes) { unpack_with(bytes, limit, dictionary)? } else { bytes.to_vec() };
    String::from_utf8(raw).map_err(|_| Error::Corrupt)
}

/// [`text_with`] for frames made without a dictionary.
pub fn text(bytes: &[u8], limit: usize) -> Result<String, Error> {
    text_with(bytes, limit, |_| None)
}
