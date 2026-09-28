//! The built page and its assets, served on both listeners (#349).
//!
//! The dashboard and the public TV answer every path their own routes do not take with the file that
//! path names under `web/dist`, or with the page itself when it names none — so the page's own routes
//! (`/training`, `/tv`) boot it on a reload. Which file that is, and as what content type, is
//! [`sv10_static`]; this is the reading of it, which is I/O and belongs here rather than there.

use super::off_runtime;
use crate::live::Shared;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use std::sync::Arc;

/// The catch-all for both listeners: the file the path names, or the page.
///
/// Reading a file blocks, so it goes on the same pool as the dashboard's other handlers — a request
/// for an asset must not stall the async workers carrying the bots' table connections. A hot swap
/// replaces the whole `web/dist` tree under a running process, so the directory is read per request
/// rather than resolved once at startup (`scripts/release.sh`).
pub(super) async fn serve(State(s): State<Arc<Shared>>, req: Request) -> Response {
    let (root, asked) = (s.config.web_dist.clone(), req.uri().path().to_string());
    let read = match off_runtime(move || {
        let file = sv10_static::file_for(&root, &asked);
        let bytes = std::fs::read(&file);
        (file, bytes)
    })
    .await
    {
        Ok(read) => read,
        Err(e) => return e.into_response(),
    };
    match read {
        (file, Ok(bytes)) => {
            let ctype = HeaderValue::from_static(sv10_static::content_type(&file));
            (StatusCode::OK, [(header::CONTENT_TYPE, ctype)], bytes).into_response()
        }
        // The file is unreadable only when the build directory is not a build (`npm run build` has
        // not run, or the tree moved). The name goes in the body so an operator who opens the
        // dashboard sees which path is missing, not only that something was.
        (file, Err(e)) => {
            tracing::warn!("{} is not readable: {e}", file.display());
            (StatusCode::NOT_FOUND, format!("{} is not a built page: {e}\n", file.display())).into_response()
        }
    }
}
