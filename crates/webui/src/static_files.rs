//! 静态资源：rust-embed 内嵌 Vue 构建产物（webui-frontend/vite build → static/）。
//! SPA 回退：未知路径回 index.html（客户端路由接管）；产物缺失时兜底 legacy-index.html。
//! 产物入库是刻意的——无 node 的机器也能 cargo build（重新生成方法见根 README）。

use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "static/"]
struct Assets;

pub async fn serve(uri: axum::http::Uri, headers: HeaderMap) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    if let Some(response) = asset_response(path, &headers) {
        return response;
    }
    // SPA 回退（/trace、/personality 等前端路由直开）
    if let Some(response) = asset_response("index.html", &headers) {
        return response;
    }
    // 构建产物缺失兜底（无 node 机器仓库快照）
    if let Some(response) = asset_response("legacy-index.html", &headers) {
        return response;
    }
    (StatusCode::NOT_FOUND, "not found").into_response()
}

fn asset_response(path: &str, request: &HeaderMap) -> Option<Response> {
    // Compress once at build time, not on the small production server per request.
    let compressed = accepts_gzip(request)
        .then(|| Assets::get(&format!("{path}.gz")))
        .flatten();
    let gzip = compressed.is_some();
    let file = compressed.or_else(|| Assets::get(path))?;
    let cache = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        // Reload the entry document after deployments, while reusing hashed assets.
        "no-cache"
    };
    let mut response = (
        [
            (header::CONTENT_TYPE, mime_of(path)),
            (header::CACHE_CONTROL, cache),
            (header::VARY, "Accept-Encoding"),
        ],
        file.data,
    )
        .into_response();
    if gzip {
        response
            .headers_mut()
            .insert(header::CONTENT_ENCODING, "gzip".parse().unwrap());
    }
    Some(response)
}

fn accepts_gzip(headers: &HeaderMap) -> bool {
    let mut gzip = None;
    let mut wildcard = None;
    for value in headers.get_all(header::ACCEPT_ENCODING).iter() {
        let Ok(value) = value.to_str() else { continue };
        for item in value.split(',') {
            let mut parts = item.split(';');
            let encoding = parts.next().unwrap_or("").trim();
            let quality = parts
                .filter_map(|part| part.trim().split_once('='))
                .find(|(key, _)| key.trim().eq_ignore_ascii_case("q"))
                .map(|(_, q)| q.trim().parse::<f32>().unwrap_or(0.0))
                .unwrap_or(1.0);
            let allowed = quality > 0.0 && quality <= 1.0;
            if encoding.eq_ignore_ascii_case("gzip") {
                gzip = Some(allowed);
            } else if encoding == "*" {
                wildcard = Some(allowed);
            }
        }
    }
    gzip.or(wildcard).unwrap_or(false)
}

fn mime_of(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("mjs") => "application/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gzip_negotiation_respects_explicit_exclusions() {
        for (encoding, expected) in [
            ("gzip, deflate, br", true),
            ("GZIP; q=0.5", true),
            ("gzip;q=0", false),
            ("gzip;q=0, *;q=1", false),
            ("br", false),
            ("*;q=0.5", true),
            ("gzip;q=invalid", false),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(header::ACCEPT_ENCODING, encoding.parse().unwrap());
            assert_eq!(accepts_gzip(&headers), expected, "{encoding}");
        }
        assert!(!accepts_gzip(&HeaderMap::new()));
    }

    #[tokio::test]
    async fn hashed_assets_use_precompressed_bytes_and_long_cache() {
        let gzip_path = Assets::iter()
            .find(|p| p.starts_with("assets/") && p.ends_with(".js.gz"))
            .expect("frontend build must include precompressed JavaScript");
        let path = gzip_path.strip_suffix(".gz").unwrap();
        for (encoding, compressed) in [("gzip", true), ("gzip;q=0, *", false)] {
            let mut headers = HeaderMap::new();
            headers.insert(header::ACCEPT_ENCODING, encoding.parse().unwrap());
            let response = serve(format!("/{path}").parse().unwrap(), headers).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers()[header::CONTENT_TYPE],
                "application/javascript; charset=utf-8"
            );
            assert_eq!(
                response.headers()[header::CACHE_CONTROL],
                "public, max-age=31536000, immutable"
            );
            assert_eq!(response.headers()[header::VARY], "Accept-Encoding");
            assert_eq!(
                response.headers().contains_key(header::CONTENT_ENCODING),
                compressed
            );
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let expected = Assets::get(if compressed { &gzip_path } else { path }).unwrap();
            assert_eq!(body.as_ref(), expected.data.as_ref());
        }
    }

    #[tokio::test]
    async fn spa_entry_requires_revalidation_after_deployments() {
        for path in ["/", "/models", "/relations"] {
            let response = serve(path.parse().unwrap(), HeaderMap::new()).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-cache");
            assert_eq!(
                response.headers()[header::CONTENT_TYPE],
                "text/html; charset=utf-8"
            );
        }
    }
}
