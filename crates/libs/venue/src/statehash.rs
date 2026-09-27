//! openpoker `state_hash` verification: drop the top-level `ts`, `table_seq`, `hand_seq` and
//! `state_hash`, serialize the rest as Python's `json.dumps(sort_keys=True, separators=(",", ":"),
//! ensure_ascii=True)` would, SHA-256 it and prefix `sha256:`. Matched 5,248/5,248 archived
//! `table_state` frames (2026-09-15).

use serde_json::Value;
use std::cell::RefCell;
use std::fmt::Write;
use sv10_digest::Sha256;

const ENVELOPE: [&str; 4] = ["ts", "table_seq", "hand_seq", "state_hash"];

thread_local! {
    /// One canonical buffer reused across frames: verification allocates nothing per frame
    /// beyond the digest comparison (0265). The buffer never escapes the thread.
    static CANON: RefCell<String> = RefCell::new(String::with_capacity(2048));
}

/// `None` when the message carries no hash; otherwise whether it matches. Compares the 32
/// digest bytes against the expected hex directly — no `format!("sha256:…")`, no hex `String`.
pub fn verify(msg: &Value) -> Option<bool> {
    let expected = msg.get("state_hash")?.as_str()?;
    let Some(expected) = parse_expected(expected) else {
        return Some(false);
    };
    let digest = CANON.with(|buf| {
        let mut buf = buf.borrow_mut();
        buf.clear();
        write_canonical(&mut buf, msg);
        Sha256::digest(buf.as_bytes())
    });
    Some(digest == expected)
}

/// Parse a `sha256:<64 hex>` expectation into digest bytes. Anything else is a mismatch, never
/// a pass: a malformed expectation must not verify.
fn parse_expected(expected: &str) -> Option<[u8; 32]> {
    let hex = expected.strip_prefix("sha256:")?;
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        out[i] = u8::from_str_radix(std::str::from_utf8(chunk).ok()?, 16).ok()?;
    }
    Some(out)
}

/// The `sha256:<hex>` hash of a message as the server computes it.
pub fn compute(msg: &Value) -> String {
    let mut out = String::with_capacity(2048);
    write_canonical(&mut out, msg);
    format!("sha256:{}", sv10_digest::hex(Sha256::digest(out.as_bytes())))
}

/// Canonical bytes of a message with the envelope (`ts`, `table_seq`, `hand_seq`, `state_hash`)
/// dropped, shared by [`compute`] and [`verify`].
fn write_canonical(out: &mut String, msg: &Value) {
    match msg {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().filter(|k| !ENVELOPE.contains(&k.as_str())).collect();
            keys.sort();
            write_object(out, keys.into_iter().map(|k| (k, &map[k])));
        }
        other => write_value(out, other),
    }
}

/// Whether a mismatching frame is stale: its `table_seq` is not newer than the last one the
/// tracker applied, so it is a replayed frame from the resync window, not a serializer bug.
/// The verdict column of [`mismatch_report`], factored out for the incident record.
pub fn is_stale(msg: &Value, last_table_seq: i64) -> bool {
    msg.get("table_seq").and_then(Value::as_i64).unwrap_or(-1) <= last_table_seq
}
/// One-line diagnosis of a mismatch, logged before the resync throws the evidence away (0265).
/// Names the verdict first — a frame whose `table_seq` is not newer than the last one the
/// tracker applied is a stale or replayed frame (resync-window artifact), not a serializer bug —
/// then a census of the snapshot's top-level fields with the two value classes that exercise
/// the fragile serialization paths (floats through `write_float`, non-ASCII through the
/// `ensure_ascii` escape), so a real divergence points at its field immediately.
pub fn mismatch_report(msg: &Value, last_table_seq: i64) -> String {
    let seq = msg.get("table_seq").and_then(Value::as_i64).unwrap_or(-1);
    let hand = msg.get("hand_seq").and_then(Value::as_i64).unwrap_or(-1);
    let verdict = if seq <= last_table_seq { "STALE" } else { "DIVERGED" };
    let mut fields = Vec::new();
    if let Some(map) = msg.as_object() {
        let mut keys: Vec<&String> = map.keys().filter(|k| !ENVELOPE.contains(&k.as_str())).collect();
        keys.sort();
        for k in keys {
            fields.push(format!("{k}={}", census(&map[k])));
        }
    }
    let canon_len = CANON.with(|buf| {
        let mut buf = buf.borrow_mut();
        buf.clear();
        write_canonical(&mut buf, msg);
        buf.len()
    });
    format!("{verdict} table_seq={seq} (last applied {last_table_seq}) hand_seq={hand} canon_len={canon_len} {}", fields.join(" "))
}

/// Compact shape of one snapshot field: type, size, and which fragile paths it exercises.
fn census(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => format!("bool({b})"),
        Value::Number(n) => {
            let float = n.as_f64().is_some_and(|f| f.fract() != 0.0 || f.abs() >= 1e15);
            format!("number{}", if float { "[float]" } else { "" })
        }
        Value::String(s) => {
            let nonascii = !s.is_ascii();
            format!("string(len {}){}", s.len(), if nonascii { "[nonascii]" } else { "" })
        }
        Value::Array(a) => {
            let mut flags = String::new();
            if a.iter().any(has_float) {
                flags.push_str("[float]");
            }
            if a.iter().any(has_nonascii) {
                flags.push_str("[nonascii]");
            }
            format!("array[{}]{flags}", a.len())
        }
        Value::Object(map) => {
            let mut flags = String::new();
            if map.values().any(has_float) {
                flags.push_str("[float]");
            }
            if map.values().any(has_nonascii) {
                flags.push_str("[nonascii]");
            }
            format!("object{{{}}}{flags}", map.len())
        }
    }
}

fn has_float(v: &Value) -> bool {
    match v {
        Value::Number(n) => n.as_f64().is_some_and(|f| f.fract() != 0.0 || f.abs() >= 1e15),
        Value::Array(a) => a.iter().any(has_float),
        Value::Object(map) => map.values().any(has_float),
        _ => false,
    }
}

fn has_nonascii(v: &Value) -> bool {
    match v {
        Value::String(s) => !s.is_ascii(),
        Value::Array(a) => a.iter().any(has_nonascii),
        Value::Object(map) => map.values().any(has_nonascii),
        _ => false,
    }
}

fn write_object<'a>(out: &mut String, entries: impl Iterator<Item = (&'a String, &'a Value)>) {
    out.push('{');
    for (i, (k, v)) in entries.enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_str(out, k);
        out.push(':');
        write_value(out, v);
    }
    out.push('}');
}

/// Python-canonical JSON (sorted keys, no whitespace, ASCII only).
pub fn canonical(v: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, v);
    out
}

fn write_value(out: &mut String, v: &Value) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                let _ = write!(out, "{i}");
            } else if let Some(u) = n.as_u64() {
                let _ = write!(out, "{u}");
            } else {
                write_float(out, n.as_f64().unwrap_or(0.0));
            }
        }
        Value::String(s) => write_str(out, s),
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(out, x);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            write_object(out, keys.into_iter().map(|k| (k, &map[k])));
        }
    }
}

fn write_str(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if !(' '..='~').contains(&c) => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Python `float.__repr__`: shortest round-trip digits, positional for exponents -4..16,
/// otherwise `d.ddde±XX`.
fn write_float(out: &mut String, f: f64) {
    if !f.is_finite() {
        out.push_str(if f.is_nan() {
            "NaN"
        } else if f > 0.0 {
            "Infinity"
        } else {
            "-Infinity"
        });
        return;
    }
    let sci = format!("{f:e}"); // e.g. "-1.25e-5", "1e16"
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let (sign, mantissa) = mantissa.strip_prefix('-').map(|m| ("-", m)).unwrap_or(("", mantissa));
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    out.push_str(sign);
    if (-4..16).contains(&exp) {
        if exp < 0 {
            out.push_str("0.");
            out.extend(std::iter::repeat_n('0', (-exp - 1) as usize));
            out.push_str(&digits);
        } else {
            let int_len = exp as usize + 1;
            if digits.len() <= int_len {
                out.push_str(&digits);
                out.extend(std::iter::repeat_n('0', int_len - digits.len()));
                out.push_str(".0");
            } else {
                out.push_str(&digits[..int_len]);
                out.push('.');
                out.push_str(&digits[int_len..]);
            }
        }
    } else {
        out.push_str(&digits[..1]);
        if digits.len() > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        let _ = write!(out, "e{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn matches_python_canonical_json() {
        // Expected strings from CPython 3 json.dumps(..., sort_keys=True, separators=(",", ":"), ensure_ascii=True).
        let v: Value = serde_json::from_str(r#"{"b":[1,-2,0.5,1e-05,100.0,1e16,123456.789,-0.0001],"a":"Sv\u00e4n \ud83d\ude00 \"q\" \\ \n\u0001\u007f","c":{"z":null,"y":true}}"#).unwrap();
        assert_eq!(
            canonical(&v),
            r#"{"a":"Sv\u00e4n \ud83d\ude00 \"q\" \\ \n\u0001\u007f","b":[1,-2,0.5,1e-05,100.0,1e+16,123456.789,-0.0001],"c":{"y":true,"z":null}}"#
        );
    }

    #[test]
    fn verifies_a_real_table_state_and_detects_tampering() {
        // A live table_state frame archived by SvanBotV9.1 (hash as sent by the server).
        let mut frame: Value = serde_json::from_str(include_str!("../tests/fixtures/table_state_hash.json")).unwrap();
        assert_eq!(verify(&frame), Some(true));
        frame["pot"] = json!(frame["pot"].as_i64().unwrap() + 1);
        assert_eq!(verify(&frame), Some(false));
        assert_eq!(verify(&json!({"type": "table_state"})), None);
    }

    #[test]
    fn malformed_expectations_never_verify() {
        let mut frame: Value = serde_json::from_str(include_str!("../tests/fixtures/table_state_hash.json")).unwrap();
        for bad in ["", "sha256:", "sha256:zzzz", "md5:00000000000000000000000000000000", "sha256:00"] {
            frame["state_hash"] = json!(bad);
            assert_eq!(verify(&frame), Some(false), "{bad} must not verify");
        }
    }

    #[test]
    fn mismatch_report_names_staleness_and_suspicious_fields() {
        let frame: Value = serde_json::from_str(include_str!("../tests/fixtures/table_state_hash.json")).unwrap();
        let seq = frame["table_seq"].as_i64().unwrap();
        let fresh = mismatch_report(&frame, seq - 1);
        assert!(fresh.starts_with("DIVERGED"), "{fresh}");
        let stale = mismatch_report(&frame, seq);
        assert!(stale.starts_with("STALE"), "{stale}");
        assert!(stale.contains("pot=number"), "{stale}");
        let floaty = json!({"table_seq": 9, "pot": 1.5, "name": "Svän", "board": [], "seats": {}});
        let report = mismatch_report(&floaty, 8);
        assert!(report.contains("pot=number[float]"), "{report}");
        assert!(report.contains("nonascii"), "{report}");
    }
}
