//! What the measurement tools read from the environment: `Params` JSON in `SIM_A`/`SIM_B` and a
//! `ModelStore` file named by `SIM_MODELS`. An input that is set but cannot be read stops the tool.
//! Falling back to the defaults made a typo measure default against default and print 0.0 (#879).

use sv10_model::model::ModelStore;
use sv10_policy::policy::Params;

/// `Params` from the JSON in `raw` (missing fields default); the defaults when it is not set.
pub fn parse_params(key: &str, raw: Option<&str>) -> Result<Params, String> {
    raw.map_or_else(|| Ok(Params::default()), |json| serde_json::from_str(json).map_err(|e| format!("{key} is not Params JSON: {e}")))
}

/// A `ModelStore` from the file at `path`; an empty one when no file is named.
pub fn parse_models(key: &str, path: Option<&str>) -> Result<ModelStore, String> {
    let Some(path) = path else { return Ok(ModelStore::default()) };
    let text = std::fs::read_to_string(path).map_err(|e| format!("{key}={path}: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("{key}={path} is not a ModelStore: {e}"))
}

/// A number from `raw` (the text of `label`); `default` when it is not given. Text that does not parse
/// is an error: a mistyped count measured something other than what was asked for.
pub fn parse_num<T: std::str::FromStr>(label: &str, raw: Option<&str>, default: T) -> Result<T, String> {
    raw.map_or(Ok(default), |text| text.parse().map_err(|_| format!("{label} is not a number: {text:?}")))
}

/// [`parse_num`] of a count that must be at least 1: zero tables, hands or salts measure nothing.
pub fn parse_count(label: &str, raw: Option<&str>, default: usize) -> Result<usize, String> {
    match parse_num(label, raw, default)? {
        0 => Err(format!("{label} must be at least 1")),
        n => Ok(n),
    }
}

fn or_exit<T>(result: Result<T, String>) -> T {
    result.unwrap_or_else(|why| {
        eprintln!("{why}");
        std::process::exit(2)
    })
}

/// [`parse_count`] of a positional argument; exits 2 naming it when it is not a count.
pub fn count(label: &str, raw: Option<&String>, default: usize) -> usize {
    or_exit(parse_count(label, raw.map(String::as_str), default))
}

/// [`parse_params`] of the environment variable `key`; exits 2 naming it when it does not parse.
pub fn params(key: &str) -> Params {
    or_exit(parse_params(key, std::env::var(key).ok().as_deref()))
}

/// [`parse_models`] of the environment variable `key`; exits 2 naming it when it cannot be read.
pub fn models(key: &str) -> ModelStore {
    or_exit(parse_models(key, std::env::var(key).ok().as_deref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_input_that_is_set_but_unreadable_is_an_error_not_the_default() {
        assert!(parse_params("SIM_B", None).is_ok());
        assert!(parse_params("SIM_B", Some("{}")).is_ok());
        let err = parse_params("SIM_B", Some("{\"call_margin\":0.005")).unwrap_err();
        assert!(err.starts_with("SIM_B is not Params JSON"), "{err}");
        assert_eq!(parse_num("SIM_SEED", None, 7), Ok(7));
        assert_eq!(parse_num("SIM_SEED", Some("9"), 7), Ok(9));
        assert!(parse_num("SIM_SEED", Some("x"), 7u64).unwrap_err().starts_with("SIM_SEED is not a number"));
        assert_eq!(parse_count("tables", Some("0"), 8), Err("tables must be at least 1".into()));
        assert!(parse_models("SIM_MODELS", None).is_ok_and(|m| m.players.is_empty()));
        let Err(err) = parse_models("SIM_MODELS", Some("/nonexistent/models.json")) else { panic!("a missing file read as a store") };
        assert!(err.starts_with("SIM_MODELS=/nonexistent/models.json"), "{err}");
    }
}
