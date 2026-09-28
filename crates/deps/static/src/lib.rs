//! Which file under a built page's directory answers a request path, and as what content type
//! (#349).
//!
//! The dashboard and the public TV both serve `web/dist`, and both did it through `tower-http`'s
//! `ServeDir`/`ServeFile` — the only thing in this workspace that reaches `mime_guess`. What the two
//! listeners need is small: an extension to a content type, a join that cannot name a file outside
//! the build directory, and `index.html` for any path that is not a file, because the page routes
//! itself. That is this crate, and it is why `tower-http` and `mime_guess` leave the tree.
//!
//! ponytail: no range requests and no conditional GETs. Nothing here streams (the build is fonts, one
//! script and one stylesheet) and nothing is fetched twice under the same URL, since the hashed
//! assets are cached for a year by the layers above and the page is a few kB. Add ranges when
//! something large or resumable is served, and `If-None-Match` when a file here is big enough that
//! re-sending it on every revalidation is worth a hash.

use std::path::{Path, PathBuf};

/// The file that answers `request_path`: the file it names under `root`, or `root/index.html` when
/// it names nothing — a missing asset, a page route, or a path that tries to leave `root`.
pub fn file_for(root: &Path, request_path: &str) -> PathBuf {
    safe_join(root, request_path).filter(|p| p.is_file()).unwrap_or_else(|| root.join("index.html"))
}

/// `root` joined with the relative segments of `request_path`, or `None` when the path is not one
/// that can only name something under `root`. A `..` is the whole point: it is refused here rather
/// than resolved and checked afterwards, so there is no step at which an escaping path exists.
fn safe_join(root: &Path, request_path: &str) -> Option<PathBuf> {
    let mut path = root.to_path_buf();
    let mut named = false;
    for segment in request_path.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." || segment.contains('\0') {
            return None;
        }
        path.push(segment);
        named = true;
    }
    named.then_some(path)
}

/// The content type `path` is served as, by extension; `application/octet-stream` when the
/// extension is one this does not know.
pub fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "txt" => "text/plain; charset=utf-8",
        "webmanifest" => "application/manifest+json",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A build directory with a page, its assets and its own subdirectory, under a root that also
    /// holds a file the build must never reach.
    struct Dist(PathBuf);

    impl Dist {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("sv10-static-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("build/assets")).unwrap();
            std::fs::write(dir.join("build/index.html"), "<!doctype html>").unwrap();
            std::fs::write(dir.join("build/assets/app-abc123.js"), "export {}").unwrap();
            std::fs::write(dir.join("secret.txt"), "not the page").unwrap();
            Dist(dir)
        }

        fn root(&self) -> PathBuf {
            self.0.join("build")
        }

        /// The file served for `path`, as it would be named in a test failure.
        fn serves(&self, path: &str) -> String {
            file_for(&self.root(), path).strip_prefix(&self.0).unwrap().display().to_string()
        }
    }

    impl Drop for Dist {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_request_path_serves_the_file_it_names_and_the_page_for_anything_else() {
        let d = Dist::new("resolve");
        assert_eq!(d.serves("/"), "build/index.html");
        assert_eq!(d.serves("/index.html"), "build/index.html");
        assert_eq!(d.serves("/assets/app-abc123.js"), "build/assets/app-abc123.js");
        // The page routes itself: a route with no file behind it is the page, not an error.
        assert_eq!(d.serves("/training"), "build/index.html");
        assert_eq!(d.serves("/assets/gone.css"), "build/index.html");
        // Nothing under the root, nothing served: a directory is not a file.
        assert_eq!(d.serves("/assets"), "build/index.html");
    }

    #[test]
    fn a_path_that_tries_to_leave_the_build_directory_serves_the_page_instead() {
        let d = Dist::new("traversal");
        for path in [
            "/../secret.txt",
            "/assets/../../secret.txt",
            "/./../../secret.txt",
            "/assets/..%2F..%2Fsecret.txt",
            "/..\\secret.txt/../../secret.txt",
        ] {
            assert_eq!(d.serves(path), "build/index.html", "{path} left the build directory");
        }
        // The control: the file is really there, one directory up from the root.
        assert!(d.0.join("secret.txt").is_file());
    }

    #[test]
    fn the_build_the_dashboard_ships_is_served_with_types_a_browser_runs() {
        for (file, want) in [
            ("build/index.html", "text/html; charset=utf-8"),
            ("build/assets/app-abc123.js", "text/javascript; charset=utf-8"),
            ("build/assets/index-abc123.css", "text/css; charset=utf-8"),
            ("build/assets/barlow-600.woff2", "font/woff2"),
            ("build/assets/barlow-600.woff", "font/woff"),
            ("build/assets/logo.svg", "image/svg+xml"),
            ("build/assets/icon-abc123.png", "image/png"),
            ("build/assets/index-abc123.js.map", "application/json"),
            // A script that arrives as octet-stream or the wrong charset is a page that does not
            // boot, so the unknown case is stated rather than left to the default arm.
            ("build/assets/README", "application/octet-stream"),
            ("build/assets/data.bin", "application/octet-stream"),
        ] {
            assert_eq!(content_type(Path::new(file)), want, "{file}");
        }
        // Case is the client's, and Windows-authored builds ship `.HTML` and `.JS`.
        assert_eq!(content_type(Path::new("build/INDEX.HTML")), "text/html; charset=utf-8");
        assert_eq!(content_type(Path::new("build/assets/APP.JS")), "text/javascript; charset=utf-8");
    }
}
