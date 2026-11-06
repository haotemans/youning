//! 带出处的人物简档与话题记忆；每日摘要仍是时间线索引，不作为简档输入。
use crate::db::SqliteExt;
use crate::db::{params, OptionalExtension};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::{BTreeMap, BTreeSet};

pub mod nightly;
pub mod processing;
pub mod refresh;

pub const PROFILE_FIELDS: usize = 5;

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileField {
    PreferredName,
    TechnicalPreferences,
    OngoingProjects,
    CommunicationStyle,
    Interests,
}

impl ProfileField {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PreferredName => "preferred_name",
            Self::TechnicalPreferences => "technical_preferences",
            Self::OngoingProjects => "ongoing_projects",
            Self::CommunicationStyle => "communication_style",
            Self::Interests => "interests",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProfileUpdate {
    pub field: ProfileField,
    pub value: String,
    pub evidence_msg_id: i64,
    pub evidence_quote: String,
}

/// 资料必须来自本轮允许范围内的本人原话。只校验来源，不宣称验证语义真实。
pub async fn update_profile(
    conn: &mut sqlx::SqliteConnection,
    person: &str,
    chat: &str,
    start: i64,
    end: i64,
    update: &ProfileUpdate,
) -> Result<bool> {
    ensure!(person != "self", "Bot 自己的回答不能生成人物简档");
    ensure!(
        (start..=end).contains(&update.evidence_msg_id),
        "人物资料出处超出本轮范围"
    );
    let value = update.value.trim();
    let quote = update.evidence_quote.trim();
    ensure!(
        !value.is_empty() && value.chars().count() <= 200,
        "人物资料长度不合法"
    );
    ensure!(
        (4..=240).contains(&quote.chars().count()),
        "人物资料须附 4–240 字原话"
    );
    ensure!(
        !crate::consolidation::is_sensitive(value) && !crate::consolidation::is_sensitive(quote),
        "人物资料含敏感内容"
    );
    let source: Option<(String, i64)> = conn.query_row(
        "SELECT COALESCE(text,''), ts FROM messages WHERE msg_id=?1 AND sender_pid=?2 AND chat_id=?3",
        params![update.evidence_msg_id, person, chat], |r| Ok((r.try_get(0)?, r.try_get(1)?)),
    ).await.optional()?;
    let (text, ts) = source.ok_or_else(|| anyhow::anyhow!("人物资料原作者或会话不匹配"))?;
    ensure!(text.contains(quote), "人物资料引文不在原消息中");
    // 更晚的原话推进出处；相同内容也推进，避免旧归纳覆盖更新的本人确认。
    Ok(conn.execute(
        "INSERT INTO person_profile_facts(person_id, field, content, source_msg_id, evidence_quote, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6)
         ON CONFLICT(person_id,field) DO UPDATE SET content=excluded.content,
           source_msg_id=excluded.source_msg_id, evidence_quote=excluded.evidence_quote, updated_at=excluded.updated_at
         WHERE excluded.source_msg_id > person_profile_facts.source_msg_id",
        params![person, update.field.as_str(), value, update.evidence_msg_id, quote, ts],
    ).await? > 0)
}

#[derive(Debug, Clone, Serialize)]
pub struct ProfileFact {
    pub field: String,
    pub content: String,
    pub source_msg_id: i64,
    pub evidence_quote: String,
    pub updated_at: i64,
}

pub async fn profile(
    conn: &mut sqlx::SqliteConnection,
    person: &str,
    cutoff: i64,
) -> Result<Vec<ProfileFact>> {
    Ok(profiles(conn, &[person.to_owned()], cutoff)
        .await?
        .remove(person)
        .unwrap_or_default())
}

/// One current portrait per identity. Directory reads share profile()'s source checks in one query.
pub async fn profiles(
    conn: &mut sqlx::SqliteConnection,
    people: &[String],
    cutoff: i64,
) -> Result<BTreeMap<String, Vec<ProfileFact>>> {
    ensure!(people.len() <= 96, "每次最多读取96个人物画像");
    let mut result: BTreeMap<String, Vec<ProfileFact>> = BTreeMap::new();
    if people.is_empty() {
        return Ok(result);
    }
    let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
        "WITH history AS (SELECT p.*,ROW_NUMBER() OVER(PARTITION BY p.person_id,p.field ORDER BY p.source_msg_id DESC,p.id DESC) AS rank
         FROM person_profile_versions p JOIN messages m ON m.msg_id=p.source_msg_id
         WHERE m.sender_pid=p.person_id AND instr(m.text,p.evidence_quote)>0 AND p.source_msg_id<=",
    );
    query.push_bind(cutoff).push(" AND p.person_id IN (");
    let mut ids = query.separated(",");
    for id in people {
        ids.push_bind(id);
    }
    ids.push_unseparated(")");
    query.push(") SELECT person_id,field,content,source_msg_id,evidence_quote,source_ts AS updated_at FROM history WHERE rank=1 ORDER BY person_id,field");
    for row in query.build().fetch_all(&mut *conn).await? {
        let facts = result.entry(row.try_get("person_id")?).or_default();
        if facts.len() < PROFILE_FIELDS {
            facts.push(ProfileFact {
                field: row.try_get("field")?,
                content: row.try_get("content")?,
                source_msg_id: row.try_get("source_msg_id")?,
                evidence_quote: row.try_get("evidence_quote")?,
                updated_at: row.try_get("updated_at")?,
            });
        }
    }
    Ok(result)
}

#[derive(Debug, Clone, Serialize)]
pub struct RecalledMemory {
    pub id: i64,
    pub owner_type: String,
    pub owner_id: String,
    pub content: String,
    pub source: String,
    pub updated_at: i64,
    pub source_chat_id: Option<String>,
    pub source_msg_id: Option<i64>,
    pub source_end_msg_id: Option<i64>,
    pub evidence_quote: Option<String>,
    pub evidence_status: String,
    pub content_truncated: bool,
}

/// 英文词 + 中文相邻双字匹配；只用于检索，不冒充语义向量或事实置信度。
pub fn query_terms(text: &str) -> Vec<String> {
    let lower: String = text
        .chars()
        .take(800)
        .flat_map(char::to_lowercase)
        .collect();
    let mut terms = Vec::new();
    let mut seen = BTreeSet::new();
    let mut push = |term: String| {
        if term.chars().count() >= 2
            && ![
                "这个", "那个", "怎么", "什么", "一下", "现在", "还是", "我们", "你们", "可以",
                "知道", "之前", "已经", "the", "and", "what", "this", "that", "with",
            ]
            .contains(&term.as_str())
            && seen.insert(term.clone())
        {
            terms.push(term);
        }
    };
    for word in lower.split(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
        if !word.is_empty() {
            push(word.to_owned());
        }
    }
    let chars: Vec<char> = lower.chars().collect();
    for pair in chars.windows(2) {
        if pair.iter().all(|c| ('\u{3400}'..='\u{9fff}').contains(c)) {
            push(pair.iter().collect());
        }
    }
    terms.truncate(16);
    terms
}

/// owner 索引限定范围，最多返回 128 个候选，再按相关性/时间排序。
pub async fn recall(
    conn: &mut sqlx::SqliteConnection,
    person: &str,
    chat: &str,
    cutoff: i64,
    query: &str,
    limit: usize,
) -> Result<Vec<RecalledMemory>> {
    recall_scoped(
        conn,
        person,
        &processing::Scope::conversation(chat, ""),
        cutoff,
        query,
        limit,
    )
    .await
}

pub async fn recall_scoped(
    conn: &mut sqlx::SqliteConnection,
    person: &str,
    scope: &processing::Scope,
    cutoff: i64,
    query: &str,
    limit: usize,
) -> Result<Vec<RecalledMemory>> {
    let chat = scope.owner_id.as_str();
    let terms = query_terms(query);
    if terms.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }
    let conditions = (0..terms.len())
        .map(|i| format!("instr(lower(content),?{})>0", i + 5))
        .collect::<Vec<_>>()
        .join(" OR ");
    let sql = format!(
        "SELECT id,owner_type,owner_id,substr(content,1,800),source,updated_at,
                source_chat_id,source_msg_id,source_end_msg_id,length(content)>800,
                (SELECT evidence_quote FROM memory_facts f WHERE f.owner_type=long_memories.owner_type AND f.owner_id=long_memories.owner_id AND f.content=long_memories.content AND f.source_msg_id=long_memories.source_msg_id LIMIT 1)
         FROM long_memories WHERE ((owner_type='person' AND owner_id=?1) OR (owner_type='chat' AND owner_id=?2))
         AND (owner_type<>'chat' OR ?4='' OR NOT EXISTS(SELECT 1 FROM messages s WHERE s.msg_id=long_memories.source_msg_id AND s.chat_id=COALESCE(long_memories.source_chat_id,long_memories.owner_id) AND s.chat_type<>?4))
         AND (source_msg_id IS NULL OR source_msg_id<=?3) AND (source_end_msg_id IS NULL OR source_end_msg_id<=?3)
         AND ({conditions}) ORDER BY updated_at DESC,id DESC LIMIT 128"
    );
    let mut binds: Vec<crate::db::Value> = vec![
        person.to_owned().into(),
        chat.to_owned().into(),
        cutoff.into(),
        scope.chat_type.clone().into(),
    ];
    binds.extend(terms.iter().cloned().map(Into::into));
    let mut st = conn.prepare(&sql).await?;
    let rows = st
        .query_map(crate::db::params_from_iter(binds), |r| {
            Ok(RecalledMemory {
                id: r.try_get(0)?,
                owner_type: r.try_get(1)?,
                owner_id: r.try_get(2)?,
                content: r.try_get(3)?,
                source: r.try_get(4)?,
                updated_at: r.try_get(5)?,
                source_chat_id: r.try_get(6)?,
                source_msg_id: r.try_get(7)?,
                source_end_msg_id: r.try_get(8)?,
                evidence_quote: r.try_get(10)?,
                evidence_status: "legacy_unverified".into(),
                content_truncated: r.try_get(9)?,
            })
        })
        .await?
        .collect::<sqlx::Result<Vec<_>>>()?;
    let mut ranked: Vec<_> = rows
        .into_iter()
        .filter(|m| !crate::consolidation::is_sensitive(&m.content))
        .map(|m| {
            let content = m.content.to_lowercase();
            let score: usize = terms
                .iter()
                .filter(|t| content.contains(t.as_str()))
                .map(|t| t.chars().count())
                .sum();
            (score, m)
        })
        .filter(|(score, _)| *score > 0)
        .collect();
    ranked.sort_by_key(|(score, m)| std::cmp::Reverse((*score, m.updated_at, m.id)));
    let mut result = Vec::new();
    let mut seen_content = BTreeSet::new();
    for (_, mut m) in ranked {
        if !seen_content.insert((m.owner_type.clone(), m.owner_id.clone(), m.content.clone())) {
            continue;
        }
        if let (Some(source), Some(source_chat)) = (m.source_msg_id, m.source_chat_id.as_deref()) {
            if let Some(end) = m.source_end_msg_id {
                m.evidence_status = "consolidation_window_not_individual_proof".into();
                if end < source {
                    m.evidence_status = "invalid_source_range".into();
                }
            } else {
                let raw: Option<(String,String)> = conn.query_row(
                    "SELECT sender_pid,COALESCE(text,'') FROM messages WHERE msg_id=?1 AND chat_id=?2",
                    params![source,source_chat], |r| Ok((r.try_get(0)?,r.try_get(1)?)),
                ).await.optional()?;
                match raw {
                    Some((sender, text))
                        if sender != "self"
                            && (m.owner_type != "person" || sender == m.owner_id) =>
                    {
                        m.evidence_status = "source_message_available_not_fact_verification".into();
                        if crate::consolidation::is_sensitive(&text) {
                            m.evidence_quote = None;
                        } else {
                            if m.evidence_quote
                                .as_ref()
                                .is_some_and(|q| !text.contains(q.as_str()))
                            {
                                m.evidence_status = "source_quote_no_longer_matches".into();
                                m.evidence_quote = None;
                            } else if m.evidence_quote.is_none() {
                                m.evidence_quote = Some(text.chars().take(240).collect());
                            }
                        }
                    }
                    _ => m.evidence_status = "source_missing_or_author_mismatch".into(),
                }
            }
        }
        result.push(m);
        if result.len() >= limit.min(10) {
            break;
        }
    }
    Ok(result)
}
