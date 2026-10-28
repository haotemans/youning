use crate::http::{domain, Http, PlatformHttpError};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use url::Url;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Bilibili,
    Douyin,
    Xiaohongshu,
    X,
}
#[derive(Clone, Debug, Serialize)]
pub struct Media {
    pub url: String,
    pub video: bool,
}
#[derive(Debug, Serialize)]
pub struct Post {
    pub platform: Platform,
    pub title: String,
    pub source: String,
    pub media: Vec<Media>,
}
pub fn platform(url: &Url) -> Result<Platform> {
    let host = url.host_str().context("缺少域名")?;
    if domain(host, "bilibili.com") || host == "b23.tv" {
        Ok(Platform::Bilibili)
    } else if domain(host, "douyin.com") || domain(host, "iesdouyin.com") {
        Ok(Platform::Douyin)
    } else if domain(host, "xiaohongshu.com")
        || domain(host, "xhslink.com")
        || domain(host, "xhslink.cn")
    {
        Ok(Platform::Xiaohongshu)
    } else if domain(host, "x.com") || domain(host, "twitter.com") || host == "t.co" {
        Ok(Platform::X)
    } else {
        bail!("仅支持 B站、抖音、小红书和 X 的作品分享链接")
    }
}
pub fn share_url(text: &str) -> Result<Url> {
    for (i, _) in text.match_indices("http") {
        let part = text[i..]
            .split(|c: char| c.is_whitespace() || matches!(c, '"' | '<' | '>'))
            .next()
            .unwrap_or("")
            .trim_end_matches(['。', '，', ',', '！', '!', '）', ')', '】', ']', ';', '；']);
        if let Ok(url) = Url::parse(part) {
            if matches!(url.scheme(), "http" | "https") && platform(&url).is_ok() {
                return Ok(url);
            }
        }
    }
    bail!("没有找到支持的平台分享链接")
}
fn string(v: &Value) -> Option<String> {
    v.as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}
fn first_url(value: &Value) -> Option<String> {
    value
        .as_array()?
        .iter()
        .filter_map(string)
        .find(|s| s.starts_with("http://") || s.starts_with("https://"))
}
fn text(v: &Value, field: &str) -> String {
    v[field].as_str().unwrap_or("").chars().take(300).collect()
}
fn push(media: &mut Vec<Media>, url: Option<String>, video: bool) {
    if let Some(url) = url {
        if !media.iter().any(|m| m.url == url) {
            media.push(Media { url, video });
        }
    }
}

/// Parse an inline JSON assignment without executing JavaScript. Bare undefined values
/// occur in XHS hydration data; normalize only outside strings.
pub fn state(html: &str, marker: &str) -> Result<Value> {
    let after = html
        .split_once(marker)
        .context("未找到作品数据，内容可能需要登录或平台规则已变化")?
        .1;
    let start = after.find('{').context("作品数据缺少 JSON 对象")?;
    let mut out = String::new();
    let mut quoted = false;
    let mut escaped = false;
    let mut depth = 0;
    let mut chars = after[start..].chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                quoted = false;
            }
            continue;
        }
        if c == '"' {
            quoted = true;
            out.push(c);
            continue;
        }
        if c == 'u' && chars.clone().take(8).collect::<String>() == "ndefined" {
            for _ in 0..8 {
                chars.next();
            }
            out.push_str("null");
            continue;
        }
        if c == '{' {
            depth += 1;
        }
        if c == '}' {
            depth -= 1;
        }
        out.push(c);
        if depth == 0 {
            break;
        }
    }
    serde_json::from_str(&out).context("平台作品数据格式变化，解析失败")
}

pub fn douyin(data: &Value, source: String) -> Result<Post> {
    let loaders = data["loaderData"]
        .as_object()
        .context("抖音没有返回作品数据")?;
    let requested = Url::parse(&source)
        .ok()
        .and_then(|url| douyin_id(&url).ok());
    let item = loaders
        .values()
        .filter_map(|p| {
            p.pointer("/videoInfoRes/item_list/0")
                .or_else(|| p.pointer("/noteInfoRes/item_list/0"))
        })
        .find(|item| {
            requested
                .as_ref()
                .is_none_or(|id| item["aweme_id"].as_str() == Some(id))
        })
        .context("抖音作品不可用或需要登录")?;
    let mut media = vec![];
    if let Some(images) = item["images"].as_array().filter(|a| !a.is_empty()) {
        for image in images {
            push(
                &mut media,
                first_url(&image["url_list"]).or_else(|| first_url(&image["download_url_list"])),
                false,
            );
        }
    } else {
        push(
            &mut media,
            first_url(&item["video"]["play_addr"]["url_list"]),
            true,
        );
    }
    Ok(Post {
        platform: Platform::Douyin,
        title: text(item, "desc"),
        source,
        media,
    })
}
/// Map `aweme/v1/web/aweme/detail/`'s `aweme_detail` JSON onto a Post.
/// Mirrors `douyin()`'s selection rules but reads the richer web-API shape:
/// images live at `images[*].url_list`, video at `video.play_addr.url_list`
/// (falling back to `video.bit_rate[*].play_addr.url_list`).
pub fn douyin_from_detail(item: &Value, source: String) -> Result<Post> {
    let mut media = vec![];
    if let Some(images) = item["images"].as_array().filter(|a| !a.is_empty()) {
        for image in images {
            push(
                &mut media,
                first_url(&image["url_list"]).or_else(|| first_url(&image["download_url_list"])),
                false,
            );
        }
    } else {
        // Douyin returns 2–3 mirror URLs for the same clip (origin + CDN + play
        // dispatcher) — they all carry the same bytes. Treat them as a primary
        // and let the first failure abort, instead of downloading the same
        // video three times (and doubling the bandwidth, which is also what
        // makes Argus angry on the third mirror).
        if let Some(url) = crate::douyin::video_play_urls(item).into_iter().next() {
            push(&mut media, Some(url), true);
        }
    }
    Ok(Post {
        platform: Platform::Douyin,
        title: text(item, "desc"),
        source,
        media,
    })
}
pub fn xiaohongshu(data: &Value, source: String) -> Result<Post> {
    let map = data
        .pointer("/note/noteDetailMap")
        .and_then(Value::as_object)
        .context("小红书没有返回笔记，可能需要登录")?;
    // Select the requested note rather than a recommendation embedded in the page.
    let url = Url::parse(&source)?;
    let requested = url
        .path_segments()
        .and_then(|s| s.filter(|s| !s.is_empty()).next_back());
    let item = requested
        .and_then(|id| map.get(id))
        .and_then(|v| v.get("note"))
        .context("页面未包含该笔记的数据")?;
    let mut media = vec![];
    if item["type"] == "video" {
        if let Some(streams) = item
            .pointer("/video/media/stream/h264")
            .and_then(Value::as_array)
        {
            let best = streams
                .iter()
                .filter(|v| v["masterUrl"].is_string())
                .max_by_key(|v| v["avgBitrate"].as_u64().unwrap_or(0));
            push(&mut media, best.and_then(|v| string(&v["masterUrl"])), true);
        }
    } else if let Some(images) = item["imageList"].as_array() {
        for image in images {
            let url = string(&image["urlDefault"]).or_else(|| string(&image["urlPre"]));
            push(&mut media, url, false);
        }
    }
    let title = if text(item, "title").is_empty() {
        text(item, "desc")
    } else {
        text(item, "title")
    };
    Ok(Post {
        platform: Platform::Xiaohongshu,
        title,
        source,
        media,
    })
}
pub fn twitter(data: &Value, source: String) -> Result<Post> {
    let mut media = vec![];
    if let Some(items) = data["mediaDetails"].as_array() {
        for item in items {
            if item["type"] == "photo" {
                push(&mut media, string(&item["media_url_https"]), false);
            } else if let Some(variants) = item
                .pointer("/video_info/variants")
                .and_then(Value::as_array)
            {
                let best = variants
                    .iter()
                    .filter(|v| v["content_type"] == "video/mp4")
                    .max_by_key(|v| v["bitrate"].as_u64().unwrap_or(0));
                push(&mut media, best.and_then(|v| string(&v["url"])), true);
            }
        }
    }
    // Older syndication responses use top-level photos/video.
    if media.is_empty() {
        if let Some(photos) = data["photos"].as_array() {
            for p in photos {
                push(&mut media, string(&p["url"]), false);
            }
        }
        if let Some(v) = data.pointer("/video/variants").and_then(Value::as_array) {
            push(
                &mut media,
                v.iter()
                    .rev()
                    .find(|v| v["type"] == "video/mp4")
                    .and_then(|v| string(&v["src"])),
                true,
            );
        }
    }
    Ok(Post {
        platform: Platform::X,
        title: text(data, "text"),
        source,
        media,
    })
}

// The public embed endpoint expects JS Number(id).toString(36) after this transform.
fn syndication_token(id: &str) -> Result<String> {
    let value = id.parse::<f64>()? / 1e15 * std::f64::consts::PI;
    let mut integer = value.floor() as u64;
    let mut digits = Vec::new();
    while integer > 0 {
        digits.push((integer % 36) as u8);
        integer /= 36;
    }
    digits.reverse();
    let mut fraction = value - value.floor();
    let mut delta = (f64::from_bits(value.to_bits() + 1) - value) / 2.0;
    for _ in 0..32 {
        if fraction < delta {
            break;
        }
        fraction *= 36.0;
        delta *= 36.0;
        let digit = fraction.floor() as u8;
        fraction -= f64::from(digit);
        digits.push(digit);
        if fraction > 1.0 - delta {
            let mut index = digits.len();
            while index > 0 {
                index -= 1;
                digits[index] += 1;
                if digits[index] < 36 {
                    break;
                }
                digits[index] = 0;
            }
            break;
        }
    }
    Ok(digits
        .into_iter()
        .filter(|d| *d != 0)
        .map(|d| char::from(if d < 10 { b'0' + d } else { b'a' + d - 10 }))
        .collect())
}

fn douyin_id(url: &Url) -> Result<String> {
    url.query_pairs()
        .find(|(k, _)| k == "modal_id" || k == "aweme_id")
        .map(|(_, v)| v.into_owned())
        .or_else(|| {
            url.path_segments()
                .and_then(|s| s.filter(|s| !s.is_empty()).next_back())
                .map(str::to_owned)
        })
        .filter(|s| (10..=24).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_digit()))
        .context("未识别到抖音作品 ID")
}

fn bilibili_video_id(url: &Url) -> Option<&str> {
    let mut parts = url.path_segments()?;
    if parts.next()? != "video" {
        return None;
    }
    let id = parts.next()?;
    ((id.starts_with("BV") && id.len() == 12 && id.bytes().all(|c| c.is_ascii_alphanumeric()))
        || (id.starts_with("av") && id.len() > 2 && id[2..].bytes().all(|c| c.is_ascii_digit())))
    .then_some(id)
}
fn bilibili_landing(url: &Url) -> bool {
    let host = url.host_str().unwrap_or_default();
    (domain(host, "bilibili.com") && bilibili_video_id(url).is_some())
        || (host == "t.bilibili.com"
            && !url.path().trim_matches('/').is_empty()
            && url
                .path()
                .trim_matches('/')
                .bytes()
                .all(|c| c.is_ascii_digit()))
}

pub async fn resolve(http: &Http, input: &str) -> Result<Post> {
    let mut url = share_url(input)?;
    let site = platform(&url)?;
    let mut short_link_trace: Vec<Url> = Vec::new();
    if matches!(url.host_str(), Some("b23.tv" | "v.douyin.com" | "t.co"))
        || url
            .host_str()
            .is_some_and(|h| domain(h, "xhslink.com") || domain(h, "xhslink.cn"))
    {
        let referer = url.origin().ascii_serialization();
        let trace = if site == Platform::Bilibili {
            http.resolve_redirects(url.as_str(), &referer, bilibili_landing)
                .await?
        } else {
            http.get_trace(url.as_str(), &referer).await?.0
        };
        let resolved = trace.last().cloned().context("短链跳转失败")?;
        if platform(&resolved)? != site {
            bail!("分享链接跳转到了其他平台");
        }
        short_link_trace = trace;
        url = resolved;
    }
    let source = url.to_string();
    let post = match site {
        Platform::Bilibili => bilibili(http, &url).await?,
        Platform::Douyin => {
            // When a short link is bounced to the login landing page, the final URL
            // has no aweme_id but an intermediate redirect usually still carries one.
            // Walk the whole trace instead of only the final hop.
            let id = douyin_id(&url).or_else(|_| {
                short_link_trace
                    .iter()
                    .rev()
                    .find_map(|u| douyin_id(u).ok())
                    .context(
                        "抖音短链未带来作品 ID，可能已跳转登录；在插件设置中填入 douyin_cookie 后重试",
                    )
            })?;
            let kind = if url.path().contains("/note/")
                || short_link_trace.iter().any(|u| u.path().contains("/note/"))
            {
                "note"
            } else {
                "video"
            };
            let source = format!("https://www.douyin.com/{kind}/{id}");
            // Preferred path: Douyin's own web API signed with a_bogus. It returns
            // structured JSON without needing a cookie or running the bdms VM.
            match crate::douyin::fetch_detail(http, &id).await {
                Ok(item) => douyin_from_detail(&item, source)?,
                Err(api_error) => {
                    // Fallback: legacy share-page scrape. Hits the bdms challenge
                    // on anonymous calls from IDC IPs, but still works when the
                    // operator configures douyin_cookie.
                    match http
                        .text(
                            &format!("https://www.iesdouyin.com/share/{kind}/{id}/"),
                            "https://www.douyin.com/",
                        )
                        .await
                    {
                        Ok((_, html)) => douyin(&state(&html, "window._ROUTER_DATA")?, source)?,
                        Err(page_error) => {
                            bail!("抖音接口失败：{api_error};分享页回退也失败：{page_error}")
                        }
                    }
                }
            }
        }
        Platform::Xiaohongshu => {
            let (_, html) = http
                .text(url.as_str(), "https://www.xiaohongshu.com/")
                .await?;
            xiaohongshu(&state(&html, "window.__INITIAL_STATE__")?, source)?
        }
        Platform::X => {
            let parts: Vec<_> = url.path_segments().context("X 链接格式错误")?.collect();
            let id = parts
                .windows(2)
                .find(|p| p[0] == "status")
                .map(|p| p[1])
                .filter(|s| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit()))
                .context("需要 X 帖子的 status 分享链接")?;
            let token = syndication_token(id)?;
            let data=http.json(&format!("https://cdn.syndication.twimg.com/tweet-result?id={id}&token={token}&lang=zh-cn"),"https://platform.twitter.com/").await?;
            twitter(&data, source)?
        }
    };
    if post.media.is_empty() {
        bail!("未取得可下载媒体：内容可能不公开、需要登录，或平台解析规则已变化");
    }
    if post.media.len() > 30 {
        bail!("单个作品超过 30 个媒体文件，请缩小下载范围");
    }
    Ok(post)
}
async fn bilibili(http: &Http, url: &Url) -> Result<Post> {
    let source = url.to_string();
    let mut media = vec![];
    if url.host_str() == Some("t.bilibili.com") {
        let id = url.path().trim_matches('/');
        if !id.bytes().all(|b| b.is_ascii_digit()) || id.is_empty() {
            bail!("B站动态链接无效");
        }
        let data = http
            .json(
                &format!("https://api.bilibili.com/x/polymer/web-dynamic/v1/detail?id={id}"),
                "https://www.bilibili.com/",
            )
            .await?;
        let dynamic = &data["data"]["item"]["modules"]["module_dynamic"];
        if let Some(items) = dynamic
            .pointer("/major/draw/items")
            .and_then(Value::as_array)
        {
            for item in items {
                push(&mut media, string(&item["src"]), false);
            }
        }
        return Ok(Post {
            platform: Platform::Bilibili,
            title: text(&dynamic["desc"], "text"),
            source,
            media,
        });
    }
    let id = bilibili_video_id(url).context("未识别到 B站 BV/av 视频号")?;
    let key = if id.starts_with("BV") {
        format!("bvid={id}")
    } else {
        format!("aid={}", &id[2..])
    };
    let view = bilibili_view_with(&key, |endpoint| async move {
        http.json(&endpoint, "https://www.bilibili.com/").await
    })
    .await?;
    if view["code"] != 0 {
        bail!("B站视频不可用，接口代码 {}", view["code"]);
    }
    let page = url
        .query_pairs()
        .find(|(k, _)| k == "p")
        .map(|(_, v)| v.parse::<usize>())
        .transpose()?
        .unwrap_or(1);
    let (cid, title) = bilibili_page(&view, page)?;
    let data=http.json(&format!("https://api.bilibili.com/x/player/playurl?{key}&cid={cid}&qn=32&fnval=1&fnver=0&platform=html5"),"https://www.bilibili.com/").await?;
    if data["code"] != 0 {
        bail!("B站播放地址不可用，接口代码 {}", data["code"]);
    }
    if let Some(parts) = data["data"]["durl"].as_array() {
        for part in parts {
            push(&mut media, string(&part["url"]), true);
        }
    }
    if media.is_empty() {
        bail!("B站未提供带音轨的 MP4，暂不支持该内容的分离音视频合并");
    }
    Ok(Post {
        platform: Platform::Bilibili,
        title,
        source,
        media,
    })
}

fn bilibili_view_fallback(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<PlatformHttpError>()
        .is_some_and(|e| e.0 == 412)
}
async fn bilibili_view_with<F, Fut>(key: &str, mut fetch: F) -> Result<Value>
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = Result<Value>>,
{
    match fetch(format!(
        "https://api.bilibili.com/x/web-interface/view?{key}"
    ))
    .await
    {
        Ok(view) => Ok(view),
        Err(error) if bilibili_view_fallback(&error) => {
            // Keep the exact video identity when the view endpoint rejects the
            // server but Bilibili's public page list and player remain usable.
            let pages = fetch(format!("https://api.bilibili.com/x/player/pagelist?{key}"))
                .await
                .context("B站视频信息接口返回 HTTP 412，备用分P接口也不可用；可在采薇设置中配置 bilibili_cookie 后重试")?;
            bilibili_view_from_pages(pages)
        }
        Err(error) => Err(error),
    }
}
fn bilibili_view_from_pages(pages: Value) -> Result<Value> {
    if pages["code"] != 0 {
        bail!("B站分P信息不可用，接口代码 {}", pages["code"]);
    }
    if !pages["data"].is_array() {
        bail!("B站分P接口缺少分P信息");
    }
    Ok(serde_json::json!({"code":0,"data":{"pages":pages["data"]}}))
}
fn bilibili_page(view: &Value, page: usize) -> Result<(u64, String)> {
    let pages = view["data"]["pages"].as_array().context("缺少分P信息")?;
    let selected = page
        .checked_sub(1)
        .and_then(|p| pages.get(p))
        .context("分P不存在")?;
    let cid = selected["cid"]
        .as_u64()
        .filter(|cid| *cid > 0)
        .context("缺少有效分P编号")?;
    let title = text(&view["data"], "title");
    Ok((
        cid,
        if title.is_empty() {
            text(selected, "part")
        } else {
            title
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[tokio::test]
    async fn bilibili_falls_back_once_for_412_and_keeps_the_video_key() {
        for key in ["bvid=BV1GJ411x7h7", "aid=170001"] {
            let mut requests = vec![];
            let view = bilibili_view_with(key, |url| {
                requests.push(url);
                std::future::ready(if requests.len() == 1 {
                    Err(PlatformHttpError(412).into())
                } else {
                    Ok(json!({"code":0,"data":[{"cid":11,"part":"第一P"},{"cid":22,"part":"第二P"}]}))
                })
            }).await.unwrap();
            assert_eq!(
                requests,
                [
                    format!("https://api.bilibili.com/x/web-interface/view?{key}"),
                    format!("https://api.bilibili.com/x/player/pagelist?{key}")
                ]
            );
            assert_eq!(bilibili_page(&view, 2).unwrap().0, 22);
        }
        for status in [403, 404, 429, 500] {
            let mut calls = 0;
            assert!(bilibili_view_with("aid=170001", |_| {
                calls += 1;
                std::future::ready(Err(PlatformHttpError(status).into()))
            })
            .await
            .is_err());
            assert_eq!(calls, 1);
        }
        let mut calls = 0;
        let view = bilibili_view_with("aid=170001", |_| {
            calls += 1;
            std::future::ready(Ok(json!({"code":-404})))
        })
        .await
        .unwrap();
        assert_eq!(view["code"], -404);
        assert_eq!(calls, 1);
    }
    #[test]
    fn bilibili_412_fallback_preserves_selected_page_and_api_errors() {
        assert!(bilibili_view_fallback(&PlatformHttpError(412).into()));
        assert!(!bilibili_view_fallback(&PlatformHttpError(403).into()));
        assert!(!bilibili_view_fallback(&anyhow::anyhow!("timeout")));
        let view = bilibili_view_from_pages(json!({"code":0,"data":[
            {"cid":11,"part":"第一P"},{"cid":22,"part":"第二P"}
        ]}))
        .unwrap();
        assert_eq!(bilibili_page(&view, 2).unwrap(), (22, "第二P".into()));
        assert!(bilibili_page(&view, 0).is_err());
        assert!(bilibili_page(&view, 3).is_err());
        assert!(bilibili_view_from_pages(json!({"code":-404,"data":[]})).is_err());
        assert!(bilibili_view_from_pages(json!({"code":0,"data":null})).is_err());
        let view = json!({"data":{"title":"原视频标题","pages":[{"cid":11,"part":"第一P"}]}});
        assert_eq!(bilibili_page(&view, 1).unwrap(), (11, "原视频标题".into()));
    }
    #[test]
    fn bilibili_short_links_stop_only_at_recognized_video_or_dynamic() {
        for url in [
            "https://www.bilibili.com/video/BV1GJ411x7h7?p=2",
            "https://m.bilibili.com/video/av170001",
            "https://t.bilibili.com/123456",
        ] {
            assert!(bilibili_landing(&Url::parse(url).unwrap()), "{url}");
        }
        for url in [
            "https://www.bilibili.com/",
            "https://www.bilibili.com/video/av",
            "https://www.bilibili.com/video/BVbad",
            "https://bilibili.com.evil.test/video/BV1GJ411x7h7",
            "https://t.bilibili.com/",
            "https://b23.tv/abc",
        ] {
            assert!(!bilibili_landing(&Url::parse(url).unwrap()), "{url}");
        }
    }
    // make serde_json::json! unqualified so the new tests can write
    // serde_json::json! explicitly (existing tests use json! via this import).
    #[test]
    fn rejects_recommended_posts_instead_of_downloading_wrong_work() {
        let xhs = json!({"note":{"noteDetailMap":{"other":{"note":{"type":"normal","imageList":[{"urlDefault":"https://cdn.test/other.jpg"}]}}}}});
        assert!(xiaohongshu(&xhs, "https://www.xiaohongshu.com/explore/requested".into()).is_err());
        let dy = json!({"loaderData":{"video_(id)/page":{"videoInfoRes":{"item_list":[{"aweme_id":"12345678901","video":{"play_addr":{"url_list":["https://cdn.test/v.mp4"]}}}]}}}});
        assert!(douyin(&dy, "https://www.douyin.com/video/99999999999".into()).is_err());
        assert_eq!(
            douyin(&dy, "https://www.douyin.com/video/12345678901".into())
                .unwrap()
                .media
                .len(),
            1
        );
        assert_eq!(
            douyin_id(
                &Url::parse("https://www.iesdouyin.com/share/video/?aweme_id=12345678901").unwrap()
            )
            .unwrap(),
            "12345678901"
        );
    }
    #[test]
    fn syndication_tokens_match_javascript() {
        assert_eq!(
            syndication_token("1578401165338976258").unwrap(),
            "3tqoys6uduo"
        );
        assert_eq!(
            syndication_token("643211948184596480").unwrap(),
            "1k4pk2jnx8j"
        );
        assert_eq!(
            syndication_token("1600649710662213632").unwrap(),
            "3vol7tqmapv"
        );
    }
    #[test]
    fn links_and_assignment_are_parsed_without_running_js() {
        assert_eq!(
            platform(&share_url("蓝玫瑰 https://xhslink.cn/o/38fDbOA9xCs 存好这段口令").unwrap())
                .unwrap(),
            Platform::Xiaohongshu
        );
        assert!(share_url("https://xhslink.cn.evil.test/o/123").is_err());
        assert_eq!(
            platform(&share_url("分享文案 https://v.douyin.com/abc/）。 复制打开").unwrap())
                .unwrap(),
            Platform::Douyin
        );
        assert!(share_url("https://x.com.evil.test/user/status/1").is_err());
        let s = state(
            r#"window.__INITIAL_STATE__ = {"a":undefined,"s":"undefined } \\\""};window.bad()"#,
            "window.__INITIAL_STATE__",
        )
        .unwrap();
        assert!(s["a"].is_null());
        assert!(s["s"].as_str().unwrap().contains("undefined"));
    }
    #[test]
    fn douyin_short_link_trace_recovers_aweme_id_from_middle_hops() {
        // Simulate: short link bounced to login landing page, but an intermediate
        // redirect still carries the real aweme_id.
        let middle =
            Url::parse("https://www.iesdouyin.com/share/video/7562505066105449738/?region=CN")
                .unwrap();
        let login = Url::parse("https://www.douyin.com/login?enter_from=share").unwrap();
        // Final hop carries nothing.
        assert!(douyin_id(&login).is_err());
        // Middle hop yields the id; that's the recovery path we want to exercise.
        assert_eq!(douyin_id(&middle).unwrap(), "7562505066105449738");
        // And the simulation of the trace scan used in resolve():
        let trace = vec![middle.clone(), login.clone()];
        let final_id = douyin_id(trace.last().unwrap());
        let recovered = final_id.or_else(|_| {
            trace
                .iter()
                .rev()
                .find_map(|u| douyin_id(u).ok())
                .context("抖音短链未带来作品 ID")
        });
        assert_eq!(recovered.unwrap(), "7562505066105449738");
        // A clean trace with no usable id anywhere aborts loudly.
        let clean: Vec<Url> = vec![login.clone()];
        let recovered = douyin_id(clean.last().unwrap()).or_else(|_| {
            clean
                .iter()
                .rev()
                .find_map(|u| douyin_id(u).ok())
                .context("抖音短链未带来作品 ID")
        });
        assert!(recovered.is_err());
    }
    #[test]
    fn douyin_id_skips_path_segments_that_look_like_ids_but_arent() {
        // Login landing paths must not be confused with an aweme_id.
        assert!(douyin_id(&Url::parse("https://www.douyin.com/login").unwrap()).is_err());
        assert!(douyin_id(&Url::parse("https://www.douyin.com/").unwrap()).is_err());
        // Legitimate long path on douyin.com still works.
        assert_eq!(
            douyin_id(&Url::parse("https://www.douyin.com/video/12345678901").unwrap()).unwrap(),
            "12345678901"
        );
        // Query-string aweme_id on an iesdouyin share URL also works.
        assert_eq!(
            douyin_id(
                &Url::parse("https://www.iesdouyin.com/share/video/?aweme_id=12345678901").unwrap()
            )
            .unwrap(),
            "12345678901"
        );
    }
    #[test]
    fn parses_douyin_aweme_detail_video() {
        // Mirror URLs for the same clip collapse to one media entry; the best
        // (douyin dispatcher) is picked by video_play_urls ordering.
        let item = serde_json::json!({
            "desc": "demo video",
            "video": {
                "play_addr": {
                    "url_list": ["https://cdn.test/low.mp4", "https://cdn.test/high.mp4"]
                }
            }
        });
        let post = douyin_from_detail(&item, "https://www.douyin.com/video/1".into()).unwrap();
        assert_eq!(post.media.len(), 1);
        assert!(post.media[0].video);
        assert_eq!(post.title, "demo video");
    }
    #[test]
    fn parses_douyin_aweme_detail_gallery() {
        let item = serde_json::json!({
            "desc": "gallery",
            "images": [
                {"url_list": ["https://cdn.test/1.jpg"]},
                {"url_list": ["https://cdn.test/2.jpg"]}
            ],
            "video": {
                "play_addr": { "url_list": ["https://cdn.test/cover.mp4"] }
            }
        });
        let post = douyin_from_detail(&item, "https://www.douyin.com/note/1".into()).unwrap();
        assert_eq!(post.media.len(), 2);
        assert!(!post.media.iter().any(|m| m.video));
    }
    #[test]
    fn parses_douyin_aweme_detail_bitrate_fallback() {
        let item = serde_json::json!({
            "video": {
                "bit_rate": [
                    { "play_addr": { "url_list": ["https://cdn.test/clear.mp4"] } }
                ]
            }
        });
        let post = douyin_from_detail(&item, "https://www.douyin.com/video/1".into()).unwrap();
        assert_eq!(post.media.len(), 1);
        assert_eq!(post.media[0].url, "https://cdn.test/clear.mp4");
    }
    #[test]
    fn parses_douyin_video_and_gallery() {
        let video = json!({"loaderData":{"video_(id)/page":{"videoInfoRes":{"item_list":[{"desc":"demo","video":{"play_addr":{"url_list":["https://cdn.test/v.mp4"]}}}]}}}});
        assert!(douyin(&video, "source".into()).unwrap().media[0].video);
        let gallery = json!({"loaderData":{"note_(id)/page":{"videoInfoRes":{"item_list":[{"images":[{"url_list":["https://cdn.test/1.jpg"]},{"url_list":["https://cdn.test/2.jpg"]}],"video":{"play_addr":{"url_list":["https://cdn.test/cover.mp4"]}}}]}}}});
        let post = douyin(&gallery, "source".into()).unwrap();
        assert_eq!(post.media.len(), 2);
        assert!(!post.media[0].video);
    }
    #[test]
    fn selects_requested_xhs_note_and_best_x_video() {
        let xhs = json!({"note":{"noteDetailMap":{"other":{"note":{"imageList":[{"urlDefault":"https://bad.test/img"}]}},"id":{"note":{"type":"normal","title":"note","imageList":[{"urlDefault":"https://cdn.test/photo"}]}}}}});
        assert_eq!(
            xiaohongshu(&xhs, "https://www.xiaohongshu.com/explore/id".into())
                .unwrap()
                .media[0]
                .url,
            "https://cdn.test/photo"
        );
        let x = json!({"text":"post","mediaDetails":[{"type":"photo","media_url_https":"https://cdn.test/photo"},{"type":"video","video_info":{"variants":[{"content_type":"application/x-mpegURL","url":"https://cdn.test/hls"},{"content_type":"video/mp4","bitrate":1,"url":"https://cdn.test/low"},{"content_type":"video/mp4","bitrate":9,"url":"https://cdn.test/high"}]}}]});
        let post = twitter(&x, "source".into()).unwrap();
        assert_eq!(post.media.len(), 2);
        assert_eq!(post.media[1].url, "https://cdn.test/high");
    }
}
