//! Douyin official web API client. Replaces the `iesdouyin.com/share/...` page
//! scrape with `douyin.com/aweme/v1/web/aweme/detail/`, which returns the work
//! metadata as structured JSON. Douyin requires every call to be signed with
//! `a_bogus` (see `crate::abogus`), which this module attaches automatically.
//!
//! Falls back gracefully: when the API returns a WAF shell or no item, callers
//! may still try the legacy share-page path.

use crate::abogus::sign;
use crate::http::Http;
use anyhow::{bail, Context, Result};
use serde_json::Value;

/// Desktop browser UA we sign under. Must match the UA used on the wire;
/// Douyin cross-checks the signature's User-Agent against the HTTP header.
pub const DESKTOP_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

const AID: &str = "6383";
const API_PATH: &str = "https://www.douyin.com/aweme/v1/web/aweme/detail/";
const REFERER: &str = "https://www.douyin.com/";

/// Query parameters `aweme/v1/web/aweme/detail/` expects, in the exact order
/// the browser sends them. Order affects `a_bogus`, so do not reorder. Douyin
/// appends more after `a_bogus` (e.g. cookie-banner state); those are covered
/// by the signature but optional here.
fn detail_query(aweme_id: &str) -> String {
    // Browser builds a long static preamble before aweme_id. The version /
    // channel bits are roughly stable; they're part of the signed surface so
    // they must match what the WAF sees. Empty marketing params stay empty.
    let params = [
        ("device_platform", "webapp"),
        ("aid", AID),
        ("channel", "channel_pc_web"),
        ("aweme_id", aweme_id),
        ("update_version_code", "170400"),
        ("pc_client_type", "1"),
        ("version_code", "170400"),
        ("version_name", "17.4.0"),
        ("cookie_enabled", "true"),
        ("screen_width", "1920"),
        ("screen_height", "1080"),
        ("browser_language", "zh-CN"),
        ("browser_platform", "Win32"),
        ("browser_name", "Chrome"),
        ("browser_version", "130.0.0.0"),
        ("browser_online", "true"),
        ("engine_name", "Blink"),
        ("engine_version", "130.0.0.0"),
        ("os_name", "Windows"),
        ("os_version", "10"),
        ("cpu_core_num", "8"),
        ("device_memory", "8"),
        ("platform", "PC"),
        ("downlink", "10"),
        ("effective_type", "4g"),
        ("round_trip_time", "50"),
        ("webid", "0"),
        ("verifyFp", ""),
        ("fp", ""),
        ("msToken", ""),
    ];
    params
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("&")
}

/// Fetch the work detail as JSON. Returns `Ok(item)` on success (the
/// `aweme_detail` object); `Err(...)` on network errors, WAF walls, or a
/// missing work — callers should treat all the same way and consider a
/// fallback.
pub async fn fetch_detail(http: &Http, aweme_id: &str) -> Result<Value> {
    let query = detail_query(aweme_id);
    let bogus = sign(DESKTOP_UA, &query);
    let url = format!("{}?{}&a_bogus={}", API_PATH, query, bogus);
    let data = http.json(&url, REFERER).await?;

    // Common healthy shapes:
    //   { "status_code": 0, "aweme_detail": {…} }
    // Common failure shapes:
    //   { "status_code": 4, "status_msg": "…" } (WAF / signature mismatch)
    //   HTML page (login wall or bdms challenge) — rejected upstream by `http.json`.
    let status = data["status_code"].as_i64().unwrap_or(-1);
    if status != 0 {
        let msg = data["status_msg"].as_str().unwrap_or("");
        bail!("抖音接口拒绝请求 status_code={} {}", status, msg);
    }
    let item = data
        .get("aweme_detail")
        .cloned()
        .filter(|v| !v.is_null())
        .context("抖音接口未返回作品详情，可能需要登录或作品被删除")?;
    Ok(item)
}

/// Pick a best playable MP4 URL out of `aweme_detail.video`.
/// Prefers `play_addr.url_list[*]` (HTTPS, no auth, available on most works),
/// falls back to `bit_rate[*].play_addr.url_list[*]` for高清 sources.
pub fn video_play_urls(item: &Value) -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    if let Some(arr) = item.pointer("/video/play_addr/url_list").and_then(Value::as_array) {
        for v in arr {
            if let Some(s) = v.as_str() {
                if s.starts_with("http") && !urls.iter().any(|u| u == s) {
                    urls.push(s.to_string());
                }
            }
        }
    }
    if urls.is_empty() {
        if let Some(tiers) = item.pointer("/video/bit_rate").and_then(Value::as_array) {
            for tier in tiers {
                if let Some(arr) = tier
                    .pointer("/play_addr/url_list")
                    .and_then(Value::as_array)
                {
                    for v in arr {
                        if let Some(s) = v.as_str() {
                            if s.starts_with("http") && !urls.iter().any(|u| u == s) {
                                urls.push(s.to_string());
                            }
                        }
                    }
                    if !urls.is_empty() {
                        break;
                    }
                }
            }
        }
    }
    urls
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_includes_aweme_id_and_aid() {
        let q = detail_query("12345678901");
        assert!(q.contains("aweme_id=12345678901"));
        assert!(q.contains("aid=6383"));
        assert!(q.contains("device_platform=webapp"));
    }

    #[test]
    fn sign_produces_alphabet_only_token() {
        let q = detail_query("12345678901");
        let token = sign(DESKTOP_UA, &q);
        assert!(!token.is_empty());
        assert!(token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+/-_=".contains(c)));
    }

    #[test]
    fn video_urls_picks_play_addr_first() {
        let item = serde_json::json!({
            "video": {
                "play_addr": { "url_list": ["https://cdn.test/a.mp4", "https://cdn.test/b.mp4"] },
                "bit_rate": [
                    { "play_addr": { "url_list": ["https://cdn.test/c.mp4"] } }
                ]
            }
        });
        assert_eq!(
            video_play_urls(&item),
            vec!["https://cdn.test/a.mp4".to_string(), "https://cdn.test/b.mp4".to_string()]
        );
    }

    #[test]
    fn video_urls_falls_back_to_bit_rate() {
        let item = serde_json::json!({
            "video": {
                "bit_rate": [
                    { "play_addr": { "url_list": ["https://cdn.test/c.mp4"] } }
                ]
            }
        });
        assert_eq!(video_play_urls(&item), vec!["https://cdn.test/c.mp4".to_string()]);
    }
}
