use anyhow::{bail, Result};
use std::sync::OnceLock;

use serde_json::Value;
use yuantuan_plugin_api::{HostServices, ToolCtx};

const RULES: &str = include_str!("../references/prompt-optimizer.md");
const STRINGS: &str = include_str!("../references/artist-strings.json");
const CATALOG: &str = include_str!("../references/nai5-community-tags.json");

pub(super) struct PreparedPrompt {
    pub prompt: String,
    pub artists: Vec<String>,
    pub optimized: bool,
}

fn catalog() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| serde_json::from_str(CATALOG).expect("bundled NAI tag catalog is valid"))
}

fn random_artist(model: &str) -> String {
    static DATA: OnceLock<Value> = OnceLock::new();
    let data = DATA.get_or_init(|| serde_json::from_str(STRINGS).expect("valid artist strings"));
    let artists = data["models"][model]
        .as_array()
        .expect("model artist strings exist");
    artists[super::random_seed() as usize % artists.len()]["prompt"]
        .as_str()
        .expect("artist string exists")
        .to_owned()
}

// Locate complete artist blocks before splitting on commas: weighted groups can
// contain several artists, and their closing :: must stay with the whole group.
fn artist_spans(text: &str) -> Vec<(usize, usize)> {
    let lower = text.to_ascii_lowercase();
    let mut spans = Vec::new();
    let mut cursor = 0;
    while let Some(offset) = lower[cursor..].find("artist:") {
        let index = cursor + offset;
        let boundary = text[..index]
            .rfind([',', '，', '\n'])
            .map_or(0, |i| i + text[i..].chars().next().unwrap().len_utf8());
        let prefix = &text[boundary..index];
        let trimmed = prefix.trim_end();
        let mut start = index;
        let mut end = None;
        if let Some(weight) = trimmed.strip_suffix("::") {
            let numeric = weight.split_whitespace().next_back().unwrap_or_default();
            if numeric.parse::<f32>().is_ok() {
                if let Some(close) = text[index..].find("::") {
                    start = boundary + weight.len() - numeric.len();
                    end = Some(index + close + 2);
                }
            }
        } else {
            let brackets = trimmed
                .chars()
                .rev()
                .take_while(|c| *c == '[' || *c == '{')
                .count();
            if brackets > 0 {
                start = boundary + trimmed.len() - brackets;
                let closing: String = trimmed[trimmed.len() - brackets..]
                    .chars()
                    .rev()
                    .map(|c| if c == '[' { ']' } else { '}' })
                    .collect();
                if let Some(close) = text[index..].find(&closing) {
                    end = Some(index + close + closing.len());
                }
            }
        }
        let end = end.unwrap_or_else(|| {
            text[index..]
                .find([',', '，', '\n', '\r'])
                .map_or(text.len(), |i| index + i)
        });
        if end > index + "artist:".len() {
            spans.push((start, end));
        }
        cursor = end.max(index + "artist:".len());
    }
    spans
}

fn valid_artist(tag: &str) -> bool {
    let lower = tag.to_lowercase();
    let Some((_, name)) = lower.split_once("artist:") else {
        return false;
    };
    !name.trim_matches(':').trim().is_empty()
        && tag.chars().count() <= 160
        && !tag.contains([',', '，', '\n', '\r', '<', '>'])
}

fn explicit_artists(text: &str) -> Vec<String> {
    let mut artists = Vec::new();
    for (start, end) in artist_spans(text) {
        let tag = text[start..end].trim();
        if !artists
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(tag))
        {
            artists.push(tag.to_owned());
        }
    }
    if artists.is_empty() {
        let lower = text.to_lowercase();
        for tag in catalog()["artists"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let name = tag.trim_start_matches("artist:");
            let found = lower.match_indices(name).any(|(offset, _)| {
                !lower[..offset]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_ascii_alphanumeric())
                    && !lower[offset + name.len()..]
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_alphanumeric())
            });
            if found {
                artists.push(tag.to_string());
            }
        }
        if let Some(aliases) = catalog()["artist_aliases"].as_object() {
            for (name, tag) in aliases {
                if text.contains(name) {
                    if let Some(tag) = tag.as_str() {
                        if !artists.iter().any(|existing| existing == tag) {
                            artists.push(tag.to_string());
                        }
                    }
                }
            }
        }
    }
    artists
}

fn scene_without_artists(text: &str) -> String {
    let mut scene = String::new();
    let mut cursor = 0;
    for (start, end) in artist_spans(text) {
        scene.push_str(&text[cursor..start]);
        cursor = end;
    }
    scene.push_str(&text[cursor..]);
    scene
        .split([',', '，', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

fn assemble(scene: &str, artists: Vec<String>, optimized: bool) -> PreparedPrompt {
    let suffix = artists.join(", ");
    // Leave space for the fixed quality tags added by the generator.
    let limit = 3936usize.saturating_sub(suffix.chars().count() + 2);
    let scene: String = scene_without_artists(scene).chars().take(limit).collect();
    let prompt = if scene.trim().is_empty() {
        suffix
    } else {
        format!("{}, {suffix}", scene.trim())
    };
    PreparedPrompt {
        prompt,
        artists,
        optimized,
    }
}

fn parse_plan(raw: &str, original: &str, fallback_artist: &str) -> Option<PreparedPrompt> {
    let raw = raw
        .trim()
        .strip_prefix("```json")
        .or_else(|| raw.trim().strip_prefix("```"))
        .unwrap_or(raw.trim())
        .trim()
        .trim_end_matches("```")
        .trim();
    let value: Value = serde_json::from_str(raw).ok()?;
    let scene = value["prompt"].as_str()?.trim();
    if scene.is_empty()
        || scene.chars().count() > 4000
        || !scene.chars().any(|c| c.is_ascii_alphabetic())
        || scene
            .chars()
            .any(|c| matches!(c, '\u{3400}'..='\u{9fff}' | '\u{f900}'..='\u{faff}'))
    {
        return None;
    }
    let lower = scene.to_ascii_lowercase();
    if [
        "sorry",
        "i cannot",
        "i can't",
        "i am unable",
        "i'm unable",
        "as an ai",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
    {
        return None;
    }
    let mut artists = explicit_artists(original);
    if artists.is_empty() {
        let quote = value["artist_quote"].as_str().unwrap_or_default().trim();
        if quote.chars().count() >= 2 && original.contains(quote) {
            artists = value["artists"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|tag| valid_artist(tag))
                .take(32)
                .map(str::to_string)
                .collect();
        }
    }
    if artists.is_empty() {
        artists.push(fallback_artist.to_string());
    }
    Some(assemble(scene, artists, true))
}

pub(super) async fn optimize(
    host: &dyn HostServices,
    ctx: &ToolCtx,
    original: &str,
    model: &str,
) -> Result<PreparedPrompt> {
    let selected = random_artist(model);
    let reference = if model == super::MODEL_5_ALIAS {
        format!(
            "NAI 5 社区精选标签（仅按需参考）：{}",
            catalog()["categories"]
        )
    } else {
        String::new()
    };
    let system = format!(
        "{RULES}\n当前模型：{}\n{reference}",
        super::model_name(model).unwrap_or("NovelAI")
    );
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(45),
        host.chat(ctx, &system, original),
    )
    .await;
    match response {
        Ok(Ok(raw)) => {
            if let Some(plan) = parse_plan(&raw, original, &selected) {
                return Ok(plan);
            }
            host.log("NAI 提示词优化失败：模型未返回有效英文提示词，已停止生图");
            bail!("提示词优化失败：模型未返回有效英文提示词；未发起生图请求")
        }
        Ok(Err(_)) => {
            host.log("NAI 提示词优化失败：文本模型调用失败，已停止生图");
            bail!("提示词优化失败：文本模型调用失败；未发起生图请求")
        }
        Err(_) => {
            host.log("NAI 提示词优化失败：文本模型调用超时，已停止生图");
            bail!("提示词优化超时；未发起生图请求，请稍后重试")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_style_is_used_only_when_no_artist_was_specified() {
        let plan = parse_plan(
            r#"{"prompt":"1girl, black hair","artists":["artist:invented"],"artist_quote":null}"#,
            "黑发女孩",
            "artist:freng",
        )
        .unwrap();
        assert_eq!(plan.artists, ["artist:freng"]);
        assert!(plan.prompt.ends_with("artist:freng"));
        assert!(!plan.prompt.contains("invented"));
        let plan = parse_plan(
            r#"{"prompt":"1girl","artists":[],"artist_quote":null}"#,
            "1girl, 0.8::artist:ask (askzy)::",
            "artist:freng",
        )
        .unwrap();
        assert_eq!(plan.artists, ["0.8::artist:ask (askzy)::"]);
        assert!(!plan.prompt.contains("freng"));
    }

    #[test]
    fn named_artist_in_chinese_is_preserved_with_source_evidence() {
        assert_eq!(explicit_artists("İgirl, ARTIST:freng"), ["ARTIST:freng"]);
        let plan = parse_plan(r#"{"prompt":"1girl, night","artists":["artist:yoneyama mai"],"artist_quote":"米山舞"}"#, "按米山舞画风画夜晚女孩", "artist:freng").unwrap();
        assert_eq!(plan.artists, ["artist:yoneyama mai"]);
        let plan = parse_plan(
            r#"{"prompt":"1girl","artists":["artist:invented"],"artist_quote":"原文不存在"}"#,
            "夜晚女孩",
            "artist:freng",
        )
        .unwrap();
        assert_eq!(plan.artists, ["artist:freng"]);
    }

    #[test]
    fn fallback_keeps_description_and_final_prompt_stays_bounded() {
        let original = format!("{}, artist:freng", "黑发".repeat(2000));
        let plan = assemble(&original, explicit_artists(&original), false);
        assert!(plan.prompt.starts_with("黑发"));
        assert!(
            super::super::with_fixed_quality(&plan.prompt)
                .chars()
                .count()
                <= 4000
        );
        for model in ["nai4.5", "nai5"] {
            let selected = random_artist(model);
            assert!(selected.matches("artist:").count() >= 3);
            assert!(selected.contains("::"));
        }
    }
    #[test]
    fn weighted_artist_strings_keep_grouping_and_all_artists() {
        let blend = "0.6::artist:a, artist:b::, 1.2::artist:c::, [[artist:d]], artist:e, artist:f";
        let original = format!("黑发御姐, {blend}, night street");
        let plan = parse_plan(
            r#"{"prompt":"1girl, night street"}"#,
            &original,
            "artist:unused",
        )
        .unwrap();
        assert_eq!(plan.artists.join(", "), blend);
        assert!(plan.prompt.ends_with(blend));
        assert_eq!(scene_without_artists(&original), "黑发御姐, night street");
        assert_eq!(
            explicit_artists("御姐 0.5::artist:a, artist:b::"),
            ["0.5::artist:a, artist:b::"]
        );
    }
    #[test]
    fn untranslated_or_non_prompt_plans_are_rejected() {
        for raw in [
            r#"{"prompt":"白发成年女性，图书馆阅读"}"#,
            r#"{"prompt":"12345"}"#,
            r#"{"error":"request declined"}"#,
            r#"{"prompt":"I cannot assist with that request."}"#,
        ] {
            assert!(parse_plan(raw, "成年女性在图书馆阅读", "artist:freng").is_none());
        }
        assert!(parse_plan(
            r#"{"prompt":"1woman, adult, white hair, red eyes, reading a book, library"}"#,
            "白发红瞳成年女性，图书馆阅读",
            "artist:freng"
        )
        .is_some());
    }
}
