//! The page: hand-written files, compiled into the binary. There is no web directory on the box
//! and no path to resolve, so no request can reach a file outside this list.

/// The file `path` names, as its content type and bytes. Each drill-down page is the same
/// document: the script reads the path and shows the view.
pub(super) fn file(path: &str) -> Option<(&'static str, &'static [u8])> {
    let html = "text/html; charset=utf-8";
    let script = "text/javascript; charset=utf-8";
    Some(match path {
        "/" | "/release" | "/scoreboard" => {
            (html, include_bytes!("../../web/index.html").as_slice())
        }
        _ if path.strip_prefix("/bot/").is_some_and(is_name) => {
            (html, include_bytes!("../../web/index.html").as_slice())
        }
        "/app.css" => (
            "text/css; charset=utf-8",
            include_bytes!("../../web/app.css").as_slice(),
        ),
        "/app.js" => (script, include_bytes!("../../web/app.js").as_slice()),
        "/fleet.js" => (script, include_bytes!("../../web/fleet.js").as_slice()),
        _ => return None,
    })
}

/// Whether `text` could be a bot's name: a short run of letters, digits, `_` and `-`.
fn is_name(text: &str) -> bool {
    (1..=64).contains(&text.len())
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}
