//! `ab BASE NEW [--suite learner|live|micro] [--repeat 10] [-- extra bench args]` — compare two
//! `bench` builds on paired, alternating repeats (0335). The port of the Python script `bench-ab` (#749).
//!
//! The machine is shared (the fleet, the analyst, the learner and other users' programs), so a
//! before and an after measured an hour apart differ by load, not by code. Each repeat runs A then B
//! (or B then A, alternating) back to back, and the verdict is on the paired ratio B/A per metric:
//! its mean and a 95% t-interval. A gain counts only when the interval excludes 1. Checksums must
//! match: a speed-only change computes the same thing, and the exit code is 2 when it does not.
//!
//! The report is what the script printed, byte for byte (`json.dumps(indent=1)`, ratios rounded to
//! four places), so the baseline tables and anything reading it keep working.

use serde_json::Value;
use std::process::{Command, ExitCode};
use sv10_stats::moments::t_interval;

type Metrics = &'static [(&'static str, bool)];

/// Metrics per suite, in report order, and whether higher is better.
const LEARNER: Metrics = &[("table_runs_per_cpu_s", true), ("table_runs_per_s", true)];
const LIVE: Metrics = &[("p50_ms", false), ("p95_ms", false), ("p99_ms", false), ("p999_ms", false), ("mean_ms", false), ("cpu_s", false)];
const MICRO: Metrics = &[
    ("eval_ns_per_hand", false),
    ("sample_ns_per_combo", false),
    ("deal_ns_per_sample_1opp", false),
    ("deal_ns_per_sample_2opp", false),
    ("reweight_ns_per_sample_1opp", false),
    ("reweight_ns_per_sample_2opp", false),
    ("equity_hu_samples_per_s", true),
    ("rng_ns_per_u64", false),
];

fn metrics(suite: &str) -> Option<Metrics> {
    match suite {
        "learner" => Some(LEARNER),
        "live" => Some(LIVE),
        "micro" => Some(MICRO),
        _ => None,
    }
}

/// Python's truthiness: a metric that is zero or null is not a ratio's denominator.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// Python's `round(x, 4)`: the decimal string is correctly rounded from the exact binary value.
fn round4(x: f64) -> f64 {
    format!("{x:.4}").parse().unwrap_or(x)
}

/// Python's `repr(float)` for the values a report holds (rounded ratios, so no exponent form).
fn py_float(x: f64) -> String {
    if x.is_nan() {
        "NaN".into()
    } else if x.is_infinite() {
        if x > 0.0 { "Infinity".into() } else { "-Infinity".into() }
    } else {
        format!("{x:?}")
    }
}

/// Python's `format(x, '.4g')`: four significant digits, trailing zeros dropped, an exponent form
/// below 1e-4 and from 1e4.
fn g4(x: f64) -> String {
    if x == 0.0 {
        return if x.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    if !x.is_finite() {
        return if x.is_nan() {
            "nan".into()
        } else if x > 0.0 {
            "inf".into()
        } else {
            "-inf".into()
        };
    }
    let trim = |s: &str| if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s.to_string() };
    let sci = format!("{x:.3e}");
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    if (-4..4).contains(&exp) {
        trim(&format!("{x:.*}", (3 - exp) as usize))
    } else {
        format!("{}e{}{:02}", trim(mantissa), if exp < 0 { '-' } else { '+' }, exp.abs())
    }
}

/// One repeat: the same suite from the base build and from the new one.
struct Pair {
    a: Value,
    b: Value,
}

/// The report text and whether every repeat's checksums matched.
fn report(suite: &str, repeat: usize, metrics: Metrics, pairs: &[Pair]) -> (String, bool) {
    let same = pairs.iter().all(|p| p.a["checksum"] == p.b["checksum"]);
    let mut rows = Vec::new();
    for &(name, higher_better) in metrics {
        let ratios: Vec<f64> = pairs
            .iter()
            .filter(|p| p.a.get(name).is_some_and(truthy) && p.b.get(name).is_some())
            .filter_map(|p| Some(p.b[name].as_f64()? / p.a[name].as_f64()?))
            .collect();
        let Some((mean, half)) = t_interval(&ratios) else { continue };
        let (lo, hi) = (mean - half, mean + half);
        let (better, worse) = if higher_better { (lo > 1.0, hi < 1.0) } else { (hi < 1.0, lo > 1.0) };
        let verdict = if better {
            "faster"
        } else if worse {
            "slower"
        } else {
            "no measurable change"
        };
        rows.push(format!(
            "  \"{name}\": {{\n   \"ratio\": {},\n   \"ci95\": [\n    {},\n    {}\n   ],\n   \"verdict\": \"{verdict}\"\n  }}",
            py_float(round4(mean)),
            py_float(round4(lo)),
            py_float(round4(hi))
        ));
    }
    let body = if rows.is_empty() { "{}".to_string() } else { format!("{{\n{}\n }}", rows.join(",\n")) };
    (format!("{{\n \"suite\": \"{suite}\",\n \"repeats\": {repeat},\n \"checksums_match\": {same},\n \"metrics\": {body}\n}}"), same)
}

fn progress(i: usize, repeat: usize, metrics: Metrics, pair: &Pair) -> String {
    let cell = |v: &Value, m: &str| g4(v.get(m).and_then(Value::as_f64).unwrap_or(0.0));
    let cells: Vec<String> = metrics.iter().map(|&(m, _)| format!("{m} {}->{}", cell(&pair.a, m), cell(&pair.b, m))).collect();
    format!("repeat {}/{repeat}: {}", i + 1, cells.join(", "))
}

struct Args {
    base: String,
    new: String,
    suite: String,
    repeat: usize,
    extra: Vec<String>,
}

fn parse(mut args: Vec<String>) -> Result<Args, String> {
    let extra = args.iter().position(|a| a == "--").map(|i| args.split_off(i)).unwrap_or_default();
    let extra = extra.into_iter().skip(1).collect();
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let usage = || "usage: ab BASE_BIN NEW_BIN [--suite learner|live|micro] [--repeat 10] [-- extra bench args]".to_string();
    let (Some(base), Some(new)) = (args.first().cloned(), args.get(1).cloned()) else { return Err(usage()) };
    let suite = value("--suite").unwrap_or_else(|| "micro".into());
    if metrics(&suite).is_none() {
        return Err(format!("unknown suite {suite}; {}", usage()));
    }
    let repeat = value("--repeat").map_or(Ok(10), |v| v.parse().map_err(|_| format!("--repeat takes a number, got {v}")))?;
    Ok(Args { base, new, suite, repeat, extra })
}

/// Run one build's suite and return its first JSON line.
fn bench(binary: &str, suite: &str, extra: &[String]) -> Result<Value, String> {
    let out = Command::new(binary).arg(suite).args(extra).output().map_err(|e| format!("{binary}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{binary} {suite} failed: {}", out.status));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| l.starts_with('{')).ok_or_else(|| format!("{binary} {suite} printed no JSON line"))?;
    serde_json::from_str(line).map_err(|e| format!("{binary}: {e}"))
}

fn run() -> Result<u8, String> {
    let args = parse(std::env::args().skip(1).collect())?;
    let suite_metrics = metrics(&args.suite).unwrap_or(MICRO);
    let mut pairs = Vec::with_capacity(args.repeat);
    for i in 0..args.repeat {
        let pair = if i % 2 == 0 {
            let a = bench(&args.base, &args.suite, &args.extra)?;
            Pair { b: bench(&args.new, &args.suite, &args.extra)?, a }
        } else {
            let b = bench(&args.new, &args.suite, &args.extra)?;
            Pair { a: bench(&args.base, &args.suite, &args.extra)?, b }
        };
        eprintln!("{}", progress(i, args.repeat, suite_metrics, &pair));
        pairs.push(pair);
    }
    let (text, same) = report(&args.suite, args.repeat, suite_metrics, &pairs);
    println!("{text}");
    if !same {
        eprintln!("CHECKSUMS DIFFER: the two builds do not compute the same thing");
        return Ok(2);
    }
    Ok(0)
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("ab: {e}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frozen output of the Python script (written with it, before it was deleted) on
    /// fifteen sets of per-repeat bench output: every suite, every t-table row, a checksum mismatch,
    /// a zero denominator, a missing metric, integer metrics and one repeat. `ab` must print the
    /// same bytes, the same progress lines and make the same exit decision.
    #[test]
    fn report_is_byte_equal_to_the_script_it_replaced() {
        let cases: Vec<Value> = serde_json::from_str(include_str!("../../tests/fixtures/ab-parity.json")).unwrap();
        assert_eq!(cases.len(), 15);
        for (n, case) in cases.iter().enumerate() {
            let suite = case["suite"].as_str().unwrap();
            let repeat = case["repeat"].as_u64().unwrap() as usize;
            let pairs: Vec<Pair> = case["pairs"].as_array().unwrap().iter().map(|p| Pair { a: p[0].clone(), b: p[1].clone() }).collect();
            let m = metrics(suite).unwrap();
            let (text, same) = report(suite, repeat, m, &pairs);
            assert_eq!(format!("{text}\n"), case["stdout"].as_str().unwrap(), "case {n} report");
            assert_eq!(if same { 0 } else { 2 }, case["exit"].as_u64().unwrap(), "case {n} exit");
            let lines: Vec<String> = pairs.iter().enumerate().map(|(i, p)| progress(i, repeat, m, p)).collect();
            let expected: Vec<&str> = case["progress"].as_array().unwrap().iter().map(|l| l.as_str().unwrap()).collect();
            assert_eq!(lines, expected, "case {n} progress");
        }
    }

    #[test]
    fn g4_matches_python_format() {
        // format(x, '.4g') in CPython 3.
        let cases = [
            (0.0, "0"),
            (1.0, "1"),
            (12345.6, "1.235e+04"),
            (9999.6, "1e+04"),
            (1234.5678, "1235"),
            (0.0001235, "0.0001235"),
            (0.00001235, "1.235e-05"),
            (0.5, "0.5"),
            (100.0, "100"),
            (123456789.0, "1.235e+08"),
            (-0.5, "-0.5"),
            (3.7, "3.7"),
            (45.2, "45.2"),
            (812.9, "812.9"),
            (1.5e6, "1.5e+06"),
            (2.5e-9, "2.5e-09"),
            (0.0123, "0.0123"),
            (99.995, "100"),
        ];
        for (x, want) in cases {
            assert_eq!(g4(x), want, "{x}");
        }
    }

    #[test]
    fn arguments_follow_the_script() {
        let args = |s: &str| parse(s.split_whitespace().map(String::from).collect());
        let a = args("old new --suite live --repeat 4 -- --threads 2").unwrap();
        assert_eq!(
            (a.base.as_str(), a.new.as_str(), a.suite.as_str(), a.repeat, a.extra),
            ("old", "new", "live", 4, vec!["--threads".to_string(), "2".to_string()])
        );
        let d = args("old new").unwrap();
        assert_eq!((d.suite.as_str(), d.repeat, d.extra.len()), ("micro", 10, 0));
        assert!(args("old").is_err() && args("old new --suite nope").is_err() && args("old new --repeat x").is_err());
    }
}
