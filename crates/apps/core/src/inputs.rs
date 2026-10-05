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

fn or_exit<T>(result: Result<T, String>) -> T {
    result.unwrap_or_else(|why| {
        eprintln!("{why}");
        std::process::exit(2)
    })
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
        assert!(parse_models("SIM_MODELS", None).is_ok_and(|m| m.players.is_empty()));
        let Err(err) = parse_models("SIM_MODELS", Some("/nonexistent/models.json")) else { panic!("a missing file read as a store") };
        assert!(err.starts_with("SIM_MODELS=/nonexistent/models.json"), "{err}");
    }
}
