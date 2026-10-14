use crate::http::{domain, Http};
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
    } else if domain(host, "xiaohongshu.com") || domain(host, "xhslink.com") {
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
            if platform(&url).is_ok() {
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

pub async fn resolve(http: &Http, input: &str) -> Result<Post> {
    let mut url = share_url(input)?;
    let site = platform(&url)?;
    if matches!(url.host_str(), Some("b23.tv" | "v.douyin.com" | "t.co"))
        || url.host_str().is_some_and(|h| domain(h, "xhslink.com"))
    {
        let (resolved, _) = http
            .get(url.as_str(), &url.origin().ascii_serialization())
            .await?;
        if platform(&resolved)? != site {
            bail!("分享链接跳转到了其他平台");
        }
        url = resolved;
    }
    let source = url.to_string();
    let post = match site {
        Platform::Bilibili => bilibili(http, &url).await?,
        Platform::Douyin => {
            let id = douyin_id(&url)?;
            let kind = if url.path().contains("/note/") {
                "note"
            } else {
                "video"
            };
            let (_, html) = http
                .text(
                    &format!("https://www.iesdouyin.com/share/{kind}/{id}/"),
                    "https://www.douyin.com/",
                )
                .await?;
            // Match the original work ID even if the short link resolves to a generic landing page.
            douyin(
                &state(&html, "window._ROUTER_DATA")?,
                format!("https://www.douyin.com/{kind}/{id}"),
            )?
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
    let id = url
        .path_segments()
        .and_then(|mut p| p.find(|p| p.starts_with("BV") || p.starts_with("av")))
        .filter(|id| id.bytes().all(|c| c.is_ascii_alphanumeric()))
        .context("未识别到 B站 BV/av 视频号")?;
    let key = if id.starts_with("BV") {
        format!("bvid={id}")
    } else {
        format!("aid={}", &id[2..])
    };
    let view = http
        .json(
            &format!("https://api.bilibili.com/x/web-interface/view?{key}"),
            "https://www.bilibili.com/",
        )
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
    let pages = view["data"]["pages"].as_array().context("缺少分P信息")?;
    let cid = page
        .checked_sub(1)
        .and_then(|p| pages.get(p))
        .and_then(|v| v["cid"].as_u64())
        .context("分P不存在")?;
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
        title: text(&view["data"], "title"),
        source,
        media,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
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
