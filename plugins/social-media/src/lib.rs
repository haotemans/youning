//! Public share-link media plugin. No third-party parser service or Python runtime.
mod abogus;
mod abogus_johnserf;
mod douyin;
mod http;
mod parse;
mod websign;
use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};
use yuantuan_plugin_api::*;

pub struct SocialMedia;
pub fn plugin() -> Arc<dyn Plugin> {
    Arc::new(SocialMedia)
}
#[async_trait]
impl Plugin for SocialMedia {
    fn presentation(&self) -> PluginPresentation {
        PluginPresentation {
            purpose: "视频与图文解析下载",
            poem: "小雅·采薇",
            verse: "采薇采薇，薇亦作止。",
            examples: vec!["/下载 分享链接", "在群聊发送 B站、抖音、小红书或 X 链接"],
        }
    }
    fn metadata(&self) -> PluginMeta {
        PluginMeta {
            id: "social-media",
            name: "采薇",
            version: env!("CARGO_PKG_VERSION"),
            api_version: API_VERSION,
            description: "实验性解析 B站、抖音、小红书和 X 的视频、图文；平台可用性见插件文档",
        }
    }
    fn default_timeout_ms(&self) -> u64 {
        120_000
    }
    fn config_fields(&self) -> Vec<ConfigField> {
        let mut fields = vec![
            ConfigField {
                key: "auto_parse",
                label: "自动解析群聊中的分享链接",
                kind: ConfigKind::Boolean,
                default: json!(true),
                secret: false,
                options: vec![],
            },
            ConfigField {
                key: "recall_source",
                label: "下载成功后撤回原分享消息（需机器人管理员权限）",
                kind: ConfigKind::Boolean,
                default: json!(true),
                secret: false,
                options: vec![],
            },
            ConfigField {
                key: "forward_threshold",
                label: "附件达到此数量时合并为聊天记录（1–30）",
                kind: ConfigKind::Integer,
                default: json!(4),
                secret: false,
                options: vec![],
            },
            ConfigField {
                key: "public_dns",
                label: "代理 Fake-IP 兼容（必要时使用 Cloudflare 公共 DNS）",
                kind: ConfigKind::Boolean,
                default: json!(true),
                secret: false,
                options: vec![],
            },
            ConfigField {
                key: "max_file_mb",
                label: "单文件上限（MiB，1–100）",
                kind: ConfigKind::Integer,
                default: json!(50),
                secret: false,
                options: vec![],
            },
            ConfigField {
                key: "max_total_mb",
                label: "单次总大小上限（MiB，1–200）",
                kind: ConfigKind::Integer,
                default: json!(100),
                secret: false,
                options: vec![],
            },
            ConfigField {
                key: "max_items",
                label: "单次最多文件数（1–30）",
                kind: ConfigKind::Integer,
                default: json!(20),
                secret: false,
                options: vec![],
            },
        ];
        for (key, label) in [
            ("bilibili_cookie", "B站 Cookie（可选）"),
            ("douyin_cookie", "抖音 Cookie（可选）"),
            ("xiaohongshu_cookie", "小红书 Cookie（可选）"),
        ] {
            fields.push(ConfigField {
                key,
                label,
                kind: ConfigKind::String,
                default: json!(""),
                secret: true,
                options: vec![],
            });
        }
        fields
    }
    fn validate_config(&self, config: &Value) -> Result<()> {
        validate_config(&self.config_fields(), config)?;
        for (key, max) in [
            ("max_file_mb", 100),
            ("max_total_mb", 200),
            ("max_items", 30),
            ("forward_threshold", 30),
        ] {
            if config
                .get(key)
                .is_some_and(|v| !v.as_i64().is_some_and(|v| (1..=max).contains(&v)))
            {
                bail!("{key} 超出允许范围");
            }
        }
        for field in self.config_fields().into_iter().filter(|f| f.secret) {
            if config[field.key]
                .as_str()
                .is_some_and(|s| s.len() > 16384 || s.contains(['\r', '\n']))
            {
                bail!("Cookie 格式不正确");
            }
        }
        Ok(())
    }
    async fn start(&self, ctx: PluginContext, config: Value) -> Result<Capabilities> {
        let cookies = [
            ("bilibili.com", "bilibili_cookie"),
            ("douyin.com", "douyin_cookie"),
            ("iesdouyin.com", "douyin_cookie"),
            ("xiaohongshu.com", "xiaohongshu_cookie"),
        ]
        .into_iter()
        .map(|(domain, key)| (domain.into(), config[key].as_str().unwrap_or("").into()))
        .collect();
        let downloader = Arc::new(Downloader {
            host: ctx.host,
            http: http::Http {
                cookies,
                public_dns: config["public_dns"].as_bool().unwrap_or(true),
            },
            file_limit: config["max_file_mb"].as_u64().unwrap_or(50) * 1048576,
            total_limit: config["max_total_mb"].as_u64().unwrap_or(100) * 1048576,
            max_items: config["max_items"].as_u64().unwrap_or(20) as usize,
            auto_parse: config["auto_parse"].as_bool().unwrap_or(true),
            delivery: DeliveryPolicy {
                recall_source: config["recall_source"].as_bool().unwrap_or(true),
                forward_threshold: Some(config["forward_threshold"].as_u64().unwrap_or(4) as usize),
            },
        });
        Ok(Capabilities {
            tools: vec![downloader.clone()],
            skills: vec![downloader],
        })
    }
}
struct Downloader {
    host: Arc<dyn HostServices>,
    http: http::Http,
    file_limit: u64,
    total_limit: u64,
    max_items: usize,
    auto_parse: bool,
    delivery: DeliveryPolicy,
}
/// Remove partially completed collections on error or cancellation, retain only successful results.
struct Outputs(Vec<PathBuf>);
impl Drop for Outputs {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}
impl Downloader {
    async fn execute(&self, input: &str) -> Result<ToolOutput> {
        self.host.log("开始解析分享链接");
        let post = parse::resolve(&self.http, input).await?;
        if post.media.len() > self.max_items {
            bail!(
                "作品包含 {} 个文件，超过配置上限 {}",
                post.media.len(),
                self.max_items
            );
        }
        let mut outputs = Outputs(vec![]);
        let mut total = 0;
        let mut items = vec![];
        let referer = match post.platform {
            parse::Platform::Bilibili => "https://www.bilibili.com/",
            parse::Platform::Douyin => "https://www.douyin.com/",
            parse::Platform::Xiaohongshu => "https://www.xiaohongshu.com/",
            parse::Platform::X => "https://x.com/",
        };
        for (i, media) in post.media.iter().enumerate() {
            if total >= self.total_limit {
                bail!("超过单次下载总大小限制");
            }
            self.host
                .log(&format!("下载文件 {}/{}", i + 1, post.media.len()));
            let path = PathBuf::from(
                self.host
                    .artifact_path(if media.video { "mp4" } else { "jpg" })
                    .await?,
            );
            outputs.0.push(path.clone());
            let bytes = self
                .http
                .download(
                    &media.url,
                    referer,
                    &path,
                    self.file_limit.min(self.total_limit - total),
                    media.video,
                )
                .await?;
            total += bytes;
            items.push(json!({"kind":if media.video {"video"} else {"image"},"bytes":bytes}));
        }
        let paths = std::mem::take(&mut outputs.0)
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let summary = format!(
            "{}\n已下载 {} 个文件，正在交付附件。",
            post.title,
            paths.len()
        );
        self.host.log("文件下载完成");
        Ok(ToolOutput {
            summary: summary.clone(),
            artifacts: paths,
            data: json!({"rendered":summary,"platform":post.platform,"source":post.source,"items":items,"total_bytes":total}),
        })
    }
}
#[async_trait]
impl Tool for Downloader {
    fn name(&self) -> &'static str {
        "social-media.download"
    }
    fn artifact_retention(&self) -> ArtifactRetention {
        ArtifactRetention::DeleteAfterSend
    }
    fn description(&self) -> &'static str {
        "下载用户指定的 B站、抖音、小红书或 X 分享链接中的视频、图文"
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"url":{"type":"string","maxLength":8192}},"required":["url"],"additionalProperties":false})
    }
    async fn call(&self, _ctx: &ToolCtx, args: Value) -> Result<ToolOutput> {
        let input = args["url"]
            .as_str()
            .filter(|s| s.len() <= 8192)
            .ok_or_else(|| anyhow::anyhow!("缺少有效分享链接"))?;
        self.execute(input).await
    }
}
#[async_trait]
impl Skill for Downloader {
    fn match_message(&self, ctx: &ToolCtx, text: &str) -> Option<Value> {
        if !self.auto_parse || ctx.chat_type != "group" || text.len() > 8192 {
            return None;
        }
        let url = parse::share_url(text).ok()?;
        Some(json!({"url":url.as_str(),"user_text":text,"automatic":true}))
    }
    fn delivery_policy(&self) -> DeliveryPolicy {
        self.delivery
    }
    fn commands(&self) -> &'static [&'static str] {
        &["/下载", "/download"]
    }
    fn artifact_retention(&self) -> ArtifactRetention {
        ArtifactRetention::DeleteAfterSend
    }
    fn def(&self) -> SkillDef {
        SkillDef{name:"social-media.download",description:"用户明确要求解析或下载 B站、抖音、小红书、X 的分享链接时，下载视频及图文并返回附件。",prompt_template:String::new(),tools:vec![],role_hint:None}
    }
    async fn invoke(&self, _ctx: &ToolCtx, slots: Value) -> Result<ToolOutput> {
        let input = slot_text(&slots, &["url", "user_text"])
            .filter(|s| s.len() <= 8192)
            .ok_or_else(|| anyhow::anyhow!("缺少有效分享链接"))?;
        self.execute(input).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct TestHost;
    #[async_trait]
    impl HostServices for TestHost {
        async fn chat(&self, _: &ToolCtx, _: &str, _: &str) -> Result<String> {
            unreachable!()
        }
        async fn read_data(&self, _: &str) -> Result<Option<Value>> {
            unreachable!()
        }
        async fn write_data(&self, _: &str, _: Value) -> Result<()> {
            unreachable!()
        }
        fn log(&self, _: &str) {}
    }
    #[tokio::test]
    async fn share_trigger_and_delivery_policy_follow_configuration() {
        let ctx = ToolCtx {
            task_id: None,
            chat_id: "42".into(),
            chat_type: "group".into(),
            sender_pid: "p_456".into(),
            locale: None,
        };
        let caps = SocialMedia
            .start(
                PluginContext {
                    host: Arc::new(TestHost),
                },
                json!({"forward_threshold":2,"recall_source":false}),
            )
            .await
            .unwrap();
        let skill = &caps.skills[0];
        let matched = skill
            .match_message(&ctx, "分享文案 https://xhslink.cn/o/38fDbOA9xCs 查看原文")
            .unwrap();
        assert_eq!(matched["url"], "https://xhslink.cn/o/38fDbOA9xCs");
        assert_eq!(
            skill.delivery_policy(),
            DeliveryPolicy {
                recall_source: false,
                forward_threshold: Some(2)
            }
        );
        assert_eq!(
            skill.artifact_retention(),
            ArtifactRetention::DeleteAfterSend
        );
        let mut private = ctx.clone();
        private.chat_type = "private".into();
        assert!(skill
            .match_message(&private, "https://xhslink.cn/o/123")
            .is_none());
        assert!(skill
            .match_message(&ctx, "https://xhslink.cn.evil.test/o/123")
            .is_none());
        let disabled = SocialMedia
            .start(
                PluginContext {
                    host: Arc::new(TestHost),
                },
                json!({"auto_parse":false}),
            )
            .await
            .unwrap();
        assert!(disabled.skills[0]
            .match_message(&ctx, "https://xhslink.cn/o/123")
            .is_none());
    }
    #[test]
    fn rejects_unsafe_config() {
        assert!(SocialMedia
            .validate_config(&json!({"max_file_mb":1000}))
            .is_err());
        assert!(SocialMedia
            .validate_config(&json!({"douyin_cookie":"a\r\nb"}))
            .is_err());
        assert!(SocialMedia
            .validate_config(&json!({"forward_threshold":0}))
            .is_err());
        assert!(SocialMedia
            .validate_config(&json!({"forward_threshold":31}))
            .is_err());
    }
}

/// Read-only resolver for diagnostics; does not download or send media.
pub async fn inspect_link(input: &str) -> Result<Value> {
    let post = parse::resolve(
        &http::Http {
            cookies: vec![],
            public_dns: true,
        },
        input,
    )
    .await?;
    Ok(
        json!({"platform":post.platform,"title":post.title,"files":post.media.len(),"videos":post.media.iter().filter(|m|m.video).count()}),
    )
}
