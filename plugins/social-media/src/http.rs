use anyhow::{bail, Context, Result};
use reqwest::{header, redirect::Policy, Response};
use std::{
    net::IpAddr,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::AsyncWriteExt;
use url::Url;

pub const UA: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 16_6 like Mac OS X) AppleWebKit/605.1.15 Version/16.6 Mobile/15E148 Safari/604.1";
pub struct Http {
    pub cookies: Vec<(String, String)>,
    pub public_dns: bool,
}
pub fn domain(host: &str, base: &str) -> bool {
    host == base || host.ends_with(&format!(".{base}"))
}
pub fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            !(v.is_private()
                || v.is_loopback()
                || v.is_link_local()
                || v.is_multicast()
                || v.is_broadcast()
                || v.is_documentation()
                || o[0] == 0
                || o[0] >= 240
                || (o[0] == 100 && (64..128).contains(&o[1]))
                || (o[0] == 198 && (o[1] == 18 || o[1] == 19)))
        }
        IpAddr::V6(v) => {
            if let Some(v4) = v.to_ipv4_mapped() {
                return public_ip(v4.into());
            }
            // Ordinary globally routed unicast only, excluding documentation ranges.
            let s = v.segments();
            s[0] & 0xe000 == 0x2000 && !(s[0] == 0x2001 && (s[1] == 0xdb8 || s[1] < 0x200))
        }
    }
}
impl Http {
    pub async fn get(&self, url: &str, referer: &str) -> Result<(Url, Response)> {
        let mut url = Url::parse(url).context("无效的链接")?;
        for _ in 0..8 {
            if !matches!(url.scheme(), "https" | "http")
                || !url.username().is_empty()
                || url.password().is_some()
            {
                bail!("仅支持不带帐号密码的 HTTP(S) 链接");
            }
            let host = url.host_str().context("链接缺少域名")?;
            let port = url.port_or_known_default().context("链接端口无效")?;
            if port != 80 && port != 443 {
                bail!("不支持该链接端口");
            }
            let mut addresses: Vec<_> = tokio::net::lookup_host((host, port))
                .await
                .context("域名解析失败")?
                .collect();
            if self.public_dns
                && !addresses.is_empty()
                && addresses.iter().all(|a| match a.ip() {
                    IpAddr::V4(v) => v.octets()[0] == 198 && matches!(v.octets()[1], 18 | 19),
                    IpAddr::V6(v) => {
                        v.segments()[0] == 0x2001 && v.segments()[1] == 2 && v.segments()[2] == 0
                    }
                })
            {
                // Fake-IP proxy DNS is not a public destination. Resolve through DoH,
                // then apply the same public-address validation and pinning as usual.
                let resolver = reqwest::Client::builder()
                    .no_proxy()
                    .redirect(Policy::none())
                    .timeout(Duration::from_secs(8))
                    .build()?;
                let mut response = resolver
                    .get("https://1.1.1.1/dns-query")
                    .header(header::ACCEPT, "application/dns-json")
                    .query(&[("name", host), ("type", "A")])
                    .send()
                    .await
                    .context("公共 DNS 查询失败")?
                    .error_for_status()?;
                let mut bytes = Vec::new();
                while let Some(chunk) = response.chunk().await? {
                    if bytes.len() + chunk.len() > 32768 {
                        bail!("DNS 响应过大");
                    }
                    bytes.extend_from_slice(&chunk);
                }
                let data: serde_json::Value = serde_json::from_slice(&bytes)?;
                addresses = data["Answer"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|v| v["type"] == 1)
                    .filter_map(|v| v["data"].as_str()?.parse::<IpAddr>().ok())
                    .map(|ip| std::net::SocketAddr::new(ip, port))
                    .collect();
            }
            if addresses.is_empty() || addresses.iter().any(|a| !public_ip(a.ip())) {
                bail!("不允许访问内网或特殊用途地址");
            }
            // Pin DNS results per request and validate every redirect, including CDN redirects.
            let client = reqwest::Client::builder()
                .no_proxy()
                .redirect(Policy::none())
                .resolve_to_addrs(host, &addresses)
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(90))
                .user_agent(if domain(host, "bilibili.com") { "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/130.0.0.0 Safari/537.36" } else { UA })
                .build()?;
            let mut request = client.get(url.clone()).header(header::REFERER, referer);
            if let Some((_, cookie)) = self
                .cookies
                .iter()
                .find(|(base, c)| !c.is_empty() && domain(host, base))
            {
                request = request.header(header::COOKIE, cookie);
            }
            let response = request
                .send()
                .await
                .map_err(|_| anyhow::anyhow!("平台请求失败或超时"))?;
            if response.status().is_redirection() {
                let target = response
                    .headers()
                    .get(header::LOCATION)
                    .context("重定向缺少地址")?
                    .to_str()?;
                url = url.join(target)?;
                continue;
            }
            if !response.status().is_success() {
                bail!(
                    "平台返回 HTTP {}，可能需要登录或稍后重试",
                    response.status().as_u16()
                );
            }
            return Ok((url, response));
        }
        bail!("分享链接重定向次数过多")
    }
    pub async fn text(&self, url: &str, referer: &str) -> Result<(Url, String)> {
        let (url, mut response) = self.get(url, referer).await?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
                bail!("解析页面超过 8 MiB");
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok((url, String::from_utf8(bytes).context("平台响应不是 UTF-8")?))
    }
    pub async fn json(&self, url: &str, referer: &str) -> Result<serde_json::Value> {
        let (_, text) = self.text(url, referer).await?;
        serde_json::from_str(&text).context("平台未返回有效 JSON，可能触发登录或验证页面")
    }
    pub async fn download(
        &self,
        url: &str,
        referer: &str,
        path: &Path,
        limit: u64,
        video: bool,
    ) -> Result<u64> {
        let (_, response) = self.get(url, referer).await?;
        write_response(response, path, limit, video).await
    }
}
async fn write_response(
    mut response: Response,
    path: &Path,
    limit: u64,
    video: bool,
) -> Result<u64> {
    if response.content_length().is_some_and(|n| n > limit) {
        bail!("文件超过下载大小限制");
    }
    let mut temporary = Partial {
        path: path.with_extension("part"),
        owned: false,
    };
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary.path)
        .await?;
    temporary.owned = true;
    let mut size = 0;
    let mut prefix = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        size += chunk.len() as u64;
        if size > limit {
            bail!("文件超过下载大小限制");
        }
        if prefix.len() < 16 {
            prefix.extend_from_slice(&chunk[..chunk.len().min(16 - prefix.len())]);
        }
        file.write_all(&chunk).await?;
    }
    let valid = if video {
        prefix.get(4..8) == Some(b"ftyp")
    } else {
        prefix.starts_with(b"\xff\xd8\xff")
            || prefix.starts_with(b"\x89PNG\r\n\x1a\n")
            || prefix.starts_with(b"GIF8")
            || (prefix.starts_with(b"RIFF") && prefix.get(8..12) == Some(b"WEBP"))
    };
    if !valid {
        bail!("下载内容不是可发送的 MP4 或图片，可能是验证页面或分离音视频流");
    }
    file.flush().await?;
    drop(file);
    tokio::fs::rename(&temporary.path, path).await?;
    Ok(size)
}
struct Partial {
    path: PathBuf,
    owned: bool,
}
impl Drop for Partial {
    fn drop(&mut self) {
        if self.owned {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn denies_private_and_mapped_addresses() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "::1",
            "::ffff:127.0.0.1",
            "fc00::1",
            "2001:db8::1",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        assert!(public_ip("1.1.1.1".parse().unwrap()));
        assert!(!domain("xiaohongshu.com.evil.com", "xiaohongshu.com"));
    }
    fn response(bytes: Vec<u8>) -> Response {
        ::http::Response::new(reqwest::Body::from(bytes)).into()
    }
    #[tokio::test]
    async fn writes_media_and_removes_invalid_or_oversized_partials() {
        let dir = std::env::temp_dir().join(format!(
            "yt-download-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("video.mp4");
        let bytes = b"\0\0\0\x18ftypmp42\0\0\0\0".to_vec();
        assert_eq!(
            write_response(response(bytes.clone()), &path, 100, true)
                .await
                .unwrap(),
            bytes.len() as u64
        );
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let invalid = dir.join("invalid.mp4");
        assert!(write_response(
            response(b"<html>login required</html>".to_vec()),
            &invalid,
            100,
            true
        )
        .await
        .is_err());
        assert!(!invalid.exists());
        assert!(!invalid.with_extension("part").exists());
        let big = dir.join("big.mp4");
        assert!(write_response(response(vec![0; 101]), &big, 100, true)
            .await
            .is_err());
        assert!(!big.exists());
        assert!(!big.with_extension("part").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
