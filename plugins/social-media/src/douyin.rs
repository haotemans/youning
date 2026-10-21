//! Douyin official web API client. Replaces the `iesdouyin.com/share/...` page
//! scrape with `douyin.com/aweme/v1/web/aweme/detail/`, which serves
//! structured JSON when the request carries the full Douyin signature stack:
//!
//! 1. a_bogus (JohnserfSeed variant — see `abogus_johnserf.rs`)
//! 2. x-secsdk-web-signature (see `websign.rs`)
//! 3. ttwid + UIFID_TEMP guest cookies harvested from `live.douyin.com`
//! 4. A random 184-char `msToken`
//! 5. The `uifid` query/header duplicate, tied to `UIFID_TEMP`
//!
//! Without (2)+(3) the WAF answers `403 Blocked by ArgusSecurityPlugin Uifid
//! Not Found` even when `a_bogus` is byte-perfect. The full chain is the only
//! observed way to get `aweme_detail` anonymously from an IDC IP.
//!
//! Falls back gracefully: callers that prefer the legacy share-page scrape
//! (when a real `douyin_cookie` is configured) can catch the error here and
//! retry with the old path.

use crate::abogus_johnserf::{generate_fingerprint, ABogusJ};
use crate::http::Http;
use crate::websign::{sign as websign, WEB_SIGNATURE_PARAM};
use anyhow::{bail, Context, Result};
use rand::Rng;
use serde_json::Value;

/// Edge-flavoured desktop UA. Must match what we sign against; Douyin
/// cross-checks the HTTP `User-Agent` header against the signature's UA.
pub const DESKTOP_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0";

const AID: &str = "6383";
const API_PATH: &str = "https://www.douyin.com/aweme/v1/web/aweme/detail/";
const LIVE_DOUYIN: &str = "https://live.douyin.com/";

/// Bootstrapping the guest identity requires reading response headers off
/// `live.douyin.com`. `Http.get_trace` hides `Set-Cookie` headers, so we make
/// one direct reqwest call here — same DNS pinning and redirect rules as the
/// shared client. We ask for headers only, no body.
pub async fn create_guest_session() -> Result<(String, String)> {
    let host = "live.douyin.com";
    let port: u16 = 443;
    let addresses: Vec<_> = tokio::net::lookup_host((host, port))
        .await
        .context("live.douyin.com DNS 解析失败")?
        .collect();
    if addresses.is_empty() || addresses.iter().any(|a| !crate::http::public_ip(a.ip())) {
        bail!("live.douyin.com DNS 结果不在公网");
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(host, &addresses)
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(DESKTOP_UA)
        .build()?;
    // follow one live.douyin.com landing; Douyin responds with ttwid + UIFID_TEMP
    // in Set-Cookie on the very first anonymous hit.
    let mut url = url::Url::parse(LIVE_DOUYIN)?;
    let mut ttwid: Option<String> = None;
    let mut uifid: Option<String> = None;
    for _ in 0..4 {
        let resp = client.get(url.clone()).send().await.context("live.douyin.com 无法连接")?;
        for value in resp.headers().get_all(reqwest::header::SET_COOKIE) {
            let value = match value.to_str() {
                Ok(v) => v,
                Err(_) => continue,
            };
            let mut parts = value.split(';');
            let pair = parts.next().unwrap_or("").trim();
            let mut kv = pair.splitn(2, '=');
            let name = kv.next().unwrap_or("").trim();
            let val = kv.next().unwrap_or("").trim();
            if name == "ttwid" && !val.is_empty() {
                ttwid = Some(val.to_string());
            } else if name == "UIFID_TEMP" && !val.is_empty() {
                uifid = Some(val.to_string());
            }
        }
        if ttwid.is_some() && uifid.is_some() {
            break;
        }
        if resp.status().is_redirection() {
            let loc = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .context("live.douyin.com 重定向缺 Location")?;
            url = url.join(loc)?;
            continue;
        }
        break;
    }
    match (ttwid, uifid) {
        (Some(t), Some(u)) => Ok((t, u)),
        _ => bail!("live.douyin.com 未发放匿名访客 cookie，Argus 访客身份不可用"),
    }
}

fn random_ms_token() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut rng = rand::thread_rng();
    (0..184)
        .map(|_| ALPHABET[rng.r#gen::<usize>() % ALPHABET.len()] as char)
        .collect()
}

fn detail_query(aweme_id: &str, uifid: &str, ms_token: &str) -> String {
    // Parameters in the exact order the browser sends them. Douyin hashes the
    // query string verbatim, and `x-secsdk-web-signature` covers ordering.
    //
    // This mirrors `dtk.platforms.douyin.params.content_detail_params` from
    // the reference plugin, which the author has battle-tested against the
    // live WAF.
    let params = [
        ("device_platform", "webapp"),
        ("aid", AID),
        ("channel", "channel_pc_web"),
        ("pc_client_type", "1"),
        ("publish_video_strategy_type", "2"),
        ("pc_libra_divert", "Windows"),
        ("version_code", "290100"),
        ("version_name", "29.1.0"),
        ("cookie_enabled", "true"),
        ("screen_width", "1920"),
        ("screen_height", "1080"),
        ("browser_language", "zh-CN"),
        ("browser_platform", "Win32"),
        ("browser_name", "Edge"),
        ("browser_version", "130.0.0.0"),
        ("browser_online", "true"),
        ("engine_name", "Blink"),
        ("engine_version", "130.0.0.0"),
        ("os_name", "Windows"),
        ("os_version", "10"),
        ("cpu_core_num", "12"),
        ("device_memory", "8"),
        ("platform", "PC"),
        ("downlink", "10"),
        ("effective_type", "4g"),
        ("round_trip_time", "100"),
        ("msToken", ms_token),
        ("aweme_id", aweme_id),
        ("uifid", uifid),
    ];
    params
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("&")
}

/// Fetch `aweme_detail` for one work using the full Johnserf abogus + websign
/// + guest-cookie chain. Returns the detail JSON object on success.
pub async fn fetch_detail(http: &Http, aweme_id: &str) -> Result<Value> {
    let (ttwid, uifid) = create_guest_session().await?;
    fetch_detail_with_session(http, aweme_id, &ttwid, &uifid).await
}

pub async fn fetch_detail_with_session(
    _http: &Http, // kept for future fallback (cookie-aware) use
    aweme_id: &str,
    ttwid: &str,
    uifid: &str,
) -> Result<Value> {
    let fingerprint = generate_fingerprint("Win32");
    let ms_token = random_ms_token();
    let query = detail_query(aweme_id, uifid, &ms_token);
    // Build the urlencoded query **once** and keep it stable across signatures.
    let ab = ABogusJ::new(DESKTOP_UA, &fingerprint);
    let (params_signed_with_abogus, _abogus) = ab.generate_abogus(&query, "");
    let final_signed = websign(&params_signed_with_abogus, uifid);

    let url = format!("{}?{}", API_PATH, final_signed.query);
    let referer = format!("https://www.douyin.com/video/{}", aweme_id);
    let cookie = format!("ttwid={}; UIFID_TEMP={};", ttwid, uifid);

    let host = "www.douyin.com";
    let port: u16 = 443;
    let addresses: Vec<_> = tokio::net::lookup_host((host, port))
        .await
        .context("www.douyin.com DNS 解析失败")?
        .collect();
    if addresses.is_empty() || addresses.iter().any(|a| !crate::http::public_ip(a.ip())) {
        bail!("www.douyin.com DNS 结果不在公网");
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(host, &addresses)
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(60))
        .user_agent(DESKTOP_UA)
        .build()?;
    let response = client
        .get(&url)
        .header(reqwest::header::ACCEPT, "application/json, text/plain, */*")
        .header(reqwest::header::ACCEPT_LANGUAGE, "zh-CN,zh;q=0.9")
        .header("sec-fetch-dest", "empty")
        .header("sec-fetch-mode", "cors")
        .header("sec-fetch-site", "same-origin")
        .header(reqwest::header::REFERER, referer)
        .header(reqwest::header::COOKIE, cookie)
        .header("uifid", uifid)
        .header(WEB_SIGNATURE_PARAM, &final_signed.signature)
        .header("x-secsdk-web-expire", &final_signed.timestamp)
        .send()
        .await
        .context("aweme/detail 请求失败或超时")?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        bail!(
            "aweme/detail HTTP {}: {}",
            status.as_u16(),
            text.chars().take(160).collect::<String>()
        );
    }
    let body = response.text().await.context("aweme/detail 响应读取失败")?;
    if body.is_empty() {
        bail!("aweme/detail 返回空 body");
    }
    let data: Value = serde_json::from_str(&body).context("aweme/detail 未返回 JSON")?;
    let status = data["status_code"].as_i64().unwrap_or(-1);
    if status != 0 {
        let msg = data["status_msg"].as_str().unwrap_or("");
        bail!("aweme/detail status_code={} {}", status, msg);
    }
    let item = data
        .get("aweme_detail")
        .cloned()
        .filter(|v| !v.is_null())
        .context("aweme/detail 未返回作品详情，可能需要登录或作品被删除")?;
    Ok(item)
}

/// Pick a best playable MP4 URL out of `aweme_detail.video`.
/// Prefers `play_addr.url_list[*]` (HTTPS, no auth, available on most works),
/// falls back to `bit_rate[*].play_addr.url_list[*]` for higher-quality tiers.
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
    fn query_includes_aweme_id_uifid_and_ms_token() {
        let q = detail_query("12345678901", "uifid-A", "ms-token-B");
        assert!(q.contains("aweme_id=12345678901"));
        assert!(q.contains("uifid=uifid-A"));
        assert!(q.contains("msToken=ms-token-B"));
        assert!(q.contains("aid=6383"));
    }

    #[test]
    fn ms_token_length_and_alphabet() {
        let t = random_ms_token();
        assert_eq!(t.len(), 184);
        assert!(t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        let t2 = random_ms_token();
        assert_ne!(t, t2);
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
}
