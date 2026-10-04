//! The web UI: built into the binary (`build.rs` takes `web/dist`), or served
//! from a directory — the one given with `genie serve --web <dir>` (a fresh
//! `npm run build:web` without rebuilding genie), or `web/dist` of the working
//! directory when the binary was built without the UI. A `--web` directory
//! without a built UI (a unit file from before the UI was built in) gives way
//! to the built-in one.
//!
//! The built-in files carry the gzip and brotli copies `build.rs` made: nothing
//! is compressed here, the bytes are picked according to `Accept-Encoding`.

use std::path::{Path, PathBuf};

use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};

/// A file of the built-in web UI: where it sits under `web/dist`, its contents,
/// and the precompressed copies kept for clients that take them.
pub struct WebAsset {
    pub path: &'static str,
    pub raw: &'static [u8],
    pub gzip: Option<&'static [u8]>,
    pub br: Option<&'static [u8]>,
}

include!(concat!(env!("OUT_DIR"), "/web_assets.rs"));

/// Where the web UI comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebUi {
    /// A built UI in a directory.
    Dir(PathBuf),
    /// The UI built into the binary.
    BuiltIn,
    /// None: why, and what to do.
    Missing(String),
}

/// The web UI to serve: a built one in `--web <dir>`, else the one built in,
/// else `web/dist` here.
pub fn resolve(web: Option<&Path>) -> WebUi {
    resolve_with(web, WEB_ASSETS)
}

fn resolve_with(web: Option<&Path>, built_in: &[WebAsset]) -> WebUi {
    match web {
        Some(dir) if dir.join("index.html").is_file() => WebUi::Dir(dir.to_path_buf()),
        _ if !built_in.is_empty() => WebUi::BuiltIn,
        Some(dir) => WebUi::Missing(format!(
            "no web UI in {}: run `npm install && npm run build:web` in the genie repository and serve its web/dist",
            dir.display()
        )),
        None if Path::new("web/dist/index.html").is_file() => WebUi::Dir(PathBuf::from("web/dist")),
        None => WebUi::Missing(
            "this genie was built without the web UI: run `npm install && npm run build:web` before `cargo build`, or serve a built one with --web <dir>"
                .into(),
        ),
    }
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, ext)| ext) {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// A file of a built-in UI for a request path. Client-side routes (`/board`,
/// `/team/G-7`) get `index.html`; a missing file under `assets/` is a 404, so a
/// page of an older build does not load HTML as a script.
///
/// The body is one of the bytes the file has — brotli, gzip or as it is — and
/// `Content-Encoding` names it. Nothing is compressed per request.
pub fn respond(assets: &'static [WebAsset], method: &Method, path: &str, headers: &HeaderMap) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let path = path.trim_start_matches('/');
    let file = match assets.iter().find(|a| a.path == path) {
        Some(f) => f,
        None if path.starts_with("assets/") => return StatusCode::NOT_FOUND.into_response(),
        None => match assets.iter().find(|a| a.path == "index.html") {
            Some(f) => f,
            None => return StatusCode::NOT_FOUND.into_response(),
        },
    };
    // Vite names assets by their contents: they never change under one name.
    let cache = if file.path.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    // Brotli first (it is the smaller one), then gzip, then the file as it is. The preference
    // only decides between the two: a copy the client accepts beats none.
    let accepted = accepted_encodings(headers);
    let (body, encoding) = match (file.br, file.gzip) {
        (Some(br), _) if accepted.br > 0.0 && accepted.br >= accepted.gzip => (br, Some("br")),
        (_, Some(gzip)) if accepted.gzip > 0.0 => (gzip, Some("gzip")),
        _ => (file.raw, None),
    };
    let mut out = HeaderMap::with_capacity(5);
    out.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type(file.path)));
    out.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    out.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    // The answer to this URL depends on `Accept-Encoding` only where there is a choice to make.
    if file.br.is_some() || file.gzip.is_some() {
        out.insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    }
    if let Some(encoding) = encoding {
        out.insert(header::CONTENT_ENCODING, HeaderValue::from_static(encoding));
    }
    (out, body).into_response()
}

/// The content codings this client accepts, with the q-values it gave them (`0.0`
/// for the ones it did not name, or named with `q=0`). `identity` needs no
/// naming: it is what is left when neither of these is accepted.
struct AcceptedEncodings {
    br: f32,
    gzip: f32,
}

fn accepted_encodings(headers: &HeaderMap) -> AcceptedEncodings {
    let mut accepted = AcceptedEncodings { br: 0.0, gzip: 0.0 };
    for value in headers.get_all(header::ACCEPT_ENCODING) {
        let Ok(value) = value.to_str() else { continue };
        for coding in value.split(',') {
            let mut parts = coding.split(';');
            let name = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
            let mut q = 1.0;
            for param in parts {
                if let Some(value) = param.trim().strip_prefix("q=") {
                    // An unparsable q is no invitation to compress.
                    q = value.trim().parse().unwrap_or(0.0);
                }
            }
            match name.as_str() {
                "br" => accepted.br = q,
                "gzip" => accepted.gzip = q,
                _ => {}
            }
        }
    }
    accepted
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The compressed bytes stand in for real ones: `respond` picks a variant and names it.
    static ASSETS: &[WebAsset] = &[
        WebAsset { path: "assets/app-1a2b.js", raw: b"console.log(1)", gzip: Some(b"console.log(1).gz"), br: Some(b"console.log(1).br") },
        WebAsset { path: "assets/app-1a2b.css", raw: b"body{}", gzip: None, br: None },
        WebAsset { path: "index.html", raw: b"<div id=\"root\"></div>", gzip: Some(b"<div id=\"root\"></div>.gz"), br: None },
        WebAsset { path: "assets/logo-9z8y.png", raw: b"\x89PNG", gzip: None, br: None },
    ];

    fn get(path: &str) -> Response {
        respond(ASSETS, &Method::GET, path, &HeaderMap::new())
    }

    fn get_accepting(path: &str, accept_encoding: &str) -> Response {
        let mut headers = HeaderMap::new();
        if !accept_encoding.is_empty() {
            headers.insert(header::ACCEPT_ENCODING, HeaderValue::from_str(accept_encoding).unwrap());
        }
        respond(ASSETS, &Method::GET, path, &headers)
    }

    async fn body(response: Response) -> Vec<u8> {
        axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap().to_vec()
    }

    #[test]
    fn files_by_path_routes_get_the_page_and_missing_assets_are_missing() {
        let js = get("/assets/app-1a2b.js");
        assert_eq!(js.status(), StatusCode::OK);
        assert_eq!(js.headers()[header::CONTENT_TYPE], "text/javascript; charset=utf-8");
        assert_eq!(js.headers()[header::CACHE_CONTROL], "public, max-age=31536000, immutable");
        assert_eq!(get("/assets/app-1a2b.css").headers()[header::CONTENT_TYPE], "text/css; charset=utf-8");
        for route in ["/", "/board", "/team/G-7", "/index.html"] {
            let page = get(route);
            assert_eq!(page.status(), StatusCode::OK, "{route}");
            assert_eq!(page.headers()[header::CONTENT_TYPE], "text/html; charset=utf-8");
            assert_eq!(page.headers()[header::CACHE_CONTROL], "no-cache", "the page is revalidated: it names the current assets");
        }
        assert_eq!(get("/assets/app-0000.js").status(), StatusCode::NOT_FOUND);
        assert_eq!(respond(ASSETS, &Method::POST, "/board", &HeaderMap::new()).status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(respond(&[], &Method::GET, "/", &HeaderMap::new()).status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn brotli_wins_over_gzip_and_gzip_stands_in_when_brotli_is_not_wanted() {
        let both = get_accepting("/assets/app-1a2b.js", "gzip, deflate, br, zstd");
        assert_eq!(both.headers()[header::CONTENT_ENCODING], "br");
        assert_eq!(both.headers()[header::VARY], "Accept-Encoding");
        assert_eq!(body(both).await, b"console.log(1).br");
        assert_eq!(get_accepting("/assets/app-1a2b.js", "gzip").headers()[header::CONTENT_ENCODING], "gzip");
        // The q-values decide, not the order.
        assert_eq!(get_accepting("/assets/app-1a2b.js", "gzip;q=0.5, br;q=0.9").headers()[header::CONTENT_ENCODING], "br");
        assert_eq!(get_accepting("/assets/app-1a2b.js", "gzip;q=0.9, br;q=0.5").headers()[header::CONTENT_ENCODING], "gzip");
        // A file without a brotli copy is sent as its gzip one.
        assert_eq!(get_accepting("/index.html", "gzip, br").headers()[header::CONTENT_ENCODING], "gzip");
    }

    #[tokio::test]
    async fn a_client_kept_from_compression_gets_the_file_as_it_is() {
        for accept in ["", "br;q=0, gzip;q=0", "gzip;q=0", "identity"] {
            let raw = get_accepting("/assets/app-1a2b.js", accept);
            assert_eq!(raw.status(), StatusCode::OK, "{accept:?}");
            assert!(raw.headers().get(header::CONTENT_ENCODING).is_none(), "{accept:?}");
            assert_eq!(body(raw).await, b"console.log(1)", "{accept:?}");
        }
        // A file with no compressed copy never varies and is never encoded.
        let png = get_accepting("/assets/logo-9z8y.png", "br, gzip");
        assert!(png.headers().get(header::CONTENT_ENCODING).is_none());
        assert!(png.headers().get(header::VARY).is_none());
        assert_eq!(png.headers()[header::CONTENT_TYPE], "image/png");
        // The brotli-only client gets the page as it is: there is no brotli copy of it.
        assert!(get_accepting("/index.html", "br").headers().get(header::CONTENT_ENCODING).is_none());
    }

    #[tokio::test]
    async fn a_client_route_gets_the_page_compressed_when_it_asks() {
        let page = get_accepting("/board", "gzip");
        assert_eq!(page.headers()[header::CONTENT_TYPE], "text/html; charset=utf-8");
        assert_eq!(page.headers()[header::CACHE_CONTROL], "no-cache");
        assert_eq!(page.headers()[header::CONTENT_ENCODING], "gzip");
        assert_eq!(body(page).await, b"<div id=\"root\"></div>.gz");
    }

    #[test]
    fn a_head_request_answers_with_the_headers_of_the_variant_it_would_send() {
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT_ENCODING, HeaderValue::from_static("gzip, br"));
        let head = respond(ASSETS, &Method::HEAD, "/assets/app-1a2b.js", &headers);
        assert_eq!(head.status(), StatusCode::OK);
        assert_eq!(head.headers()[header::CONTENT_ENCODING], "br");
        assert_eq!(head.headers()[header::CONTENT_TYPE], "text/javascript; charset=utf-8");
    }

    #[test]
    fn a_built_directory_given_wins_and_an_empty_one_gives_way_to_the_built_in_ui() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(resolve_with(Some(dir.path()), ASSETS), WebUi::BuiltIn);
        assert!(matches!(resolve_with(Some(dir.path()), &[]), WebUi::Missing(m) if m.contains("no web UI in")));
        std::fs::write(dir.path().join("index.html"), "<div id=\"root\"></div>").unwrap();
        assert_eq!(resolve_with(Some(dir.path()), ASSETS), WebUi::Dir(dir.path().to_path_buf()));
        assert_eq!(resolve_with(None, ASSETS), WebUi::BuiltIn);
    }
}
