//! The page: hand-written files, compiled into the binary. There is no web directory on the box
//! and no path to resolve, so no request can reach a file outside this list.

/// The file `path` names, as its content type and bytes. Each drill-down page is the same
/// document: the script reads the path and shows the view.
pub(super) fn file(path: &str) -> Option<(&'static str, &'static [u8])> {
    let html = "text/html; charset=utf-8";
    let script = "text/javascript; charset=utf-8";
    Some(match path {
        "/" | "/release" | "/scoreboard" | "/dossier" => {
            (html, include_bytes!("../../web/index.html").as_slice())
        }
        _ if path.strip_prefix("/bot/").is_some_and(is_hand_path) => {
            (html, include_bytes!("../../web/index.html").as_slice())
        }
        "/app.css" => (
            "text/css; charset=utf-8",
            include_bytes!("../../web/app.css").as_slice(),
        ),
        "/app.js" => (script, include_bytes!("../../web/app.js").as_slice()),
        "/fleet.js" => (script, include_bytes!("../../web/fleet.js").as_slice()),
        "/dom.js" => (script, include_bytes!("../../web/dom.js").as_slice()),
        "/replay.js" => (script, include_bytes!("../../web/replay.js").as_slice()),
        "/dossier.js" => (script, include_bytes!("../../web/dossier.js").as_slice()),
        _ => return None,
    })
}

/// Whether `rest` is a bot's name, a bot's name and one of its hands, or a bot's dossier:
/// `/bot/Alpha`, `/bot/Alpha/hand/h-1` or `/bot/Alpha/dossier`. Nothing else after `/bot/` is a
/// page.
fn is_hand_path(rest: &str) -> bool {
    if let Some(bot) = rest.strip_suffix("/dossier") {
        return is_name(bot);
    }
    match rest.split_once("/hand/") {
        Some((bot, hand)) => is_name(bot) && is_name(hand),
        None => is_name(rest),
    }
}

/// Whether `text` could be a bot's name: a short run of letters, digits, `_` and `-`. The page
/// route and the JSON routes both refuse anything else.
pub(super) fn is_name(text: &str) -> bool {
    (1..=64).contains(&text.len())
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}
