//! The exported page: resolve a URL to a file inside the web root, and refuse anything that
//! escapes it.

use std::path::{Path, PathBuf};

use tiny_http::{Header, Request, Response, StatusCode};

/// Serves `url` from `root`, or a 404. A missing export is a 503: the page is not installed,
/// which is a different problem from a wrong URL.
pub fn serve(request: Request, root: &Path, url: &str) {
    if !root.is_dir() {
        let _ = request.respond(
            Response::from_string(
                "the web export is not installed; run `bun run --cwd web build` and set SVANBOT_WEB\n",
            )
            .with_status_code(StatusCode(503))
            .with_header(header("Content-Type", "text/plain; charset=utf-8")),
        );
        return;
    }
    let Some(path) = resolve(root, url) else {
        let _ = request.respond(
            Response::from_string("not found\n")
                .with_status_code(StatusCode(404))
                .with_header(header("Content-Type", "text/plain; charset=utf-8")),
        );
        return;
    };
    let Ok(bytes) = std::fs::read(&path) else {
        let _ = request.respond(Response::from_string("not found\n").with_status_code(404));
        return;
    };
    let cache = if url.starts_with("/_next/static/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let response = Response::from_data(bytes)
        .with_header(content_type(&path))
        .with_header(header("Cache-Control", cache));
    let _ = request.respond(response);
}

/// The file `url` names under `root`, if it is inside `root` once symlinks are resolved.
fn resolve(root: &Path, url: &str) -> Option<PathBuf> {
    let path = url.split('?').next().unwrap_or("/");
    if path.contains('\0') {
        return None;
    }
    let relative = path.trim_start_matches('/');
    if relative
        .split('/')
        .any(|segment| segment == ".." || segment == "." || segment.contains('\\'))
    {
        return None;
    }
    let mut candidate = root.join(relative);
    if relative.is_empty() || path.ends_with('/') {
        candidate.push("index.html");
    } else if candidate.extension().is_none() {
        // The export can be read as `/page` or `/page/`: Next writes `page.html` or
        // `page/index.html` depending on `trailingSlash`.
        let html = candidate.with_extension("html");
        if html.is_file() {
            candidate = html;
        } else {
            candidate.push("index.html");
        }
    }
    let root = root.canonicalize().ok()?;
    let candidate = candidate.canonicalize().ok()?;
    candidate.starts_with(&root).then_some(candidate)
}

fn content_type(path: &Path) -> Header {
    let value = match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    };
    header("Content-Type", value)
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("a literal header")
}
