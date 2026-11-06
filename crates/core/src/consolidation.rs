//! 夜间归纳（架构文档七章 + docs/reference/data-model.md 十一章更新语义）：
//! 调度：config [consolidation]（enabled / daily_time="HH:MM" / run_on_startup 调试项），
//! tokio 定时任务 + AtomicBool 单实例锁（夜间归纳重跑会脏数据，见 runtime-design 三串行）。
//!
//! 执行：按会话类型和编号读取未归纳消息的连续前缀，每批最多500条/32000字，并保留分日来源。
//! - @关系统计纯 SQL：窗口内 mentions 对计数 → relationship_events(kind='mention',
//!   delta_familiar=0.01×count 单边单次封顶 0.05) → edges 增量累加 clamp(0..1)；
//!   @bot 计入 self→该 user 的 familiar
//! - LLM 提炼（decision 角色）：chat_summary / person_facts / group_facts / rel_events
//! - 写入：带出处事实、按批次追加日记、兼容长期记忆、rel_events→edges(trust)
//! - 敏感拒收：写入前每条 fact/summary/evidence 过 is_sensitive，命中丢弃记日志
//!
//! 批次结果与游标同事务提交；模型失败不消费未处理消息。@统计使用独立游标，不重复累计。

use crate::db::params;
use crate::db::{OptionalExtension, SqliteExt};
use crate::event::{ChatConsolidationOutcome, ConsolidationDonePayload, Event, EventBus};
use crate::llm::{LlmGateway, Role};
use crate::memory::processing;
use anyhow::Result;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

static RUNNING: AtomicBool = AtomicBool::new(false);

const WINDOW: i64 = 500;
const MENTION_DELTA_PER: f64 = 0.01;
const MENTION_DELTA_CAP: f64 = 0.05;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ConsolidationCfg {
    pub enabled: bool,
    pub daily_time: String,
    pub run_on_startup: bool,
    pub portrait_daily_limit: usize,
    pub portrait_interval_secs: u64,
    pub conversation_daily_limit: usize,
}

impl Default for ConsolidationCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            daily_time: "03:00".into(),
            run_on_startup: false,
            portrait_daily_limit: 10,
            portrait_interval_secs: 60,
            conversation_daily_limit: 20,
        }
    }
}

impl ConsolidationCfg {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            parse_hhmm(&self.daily_time).is_some(),
            "每日执行时刻须为 HH:MM"
        );
        anyhow::ensure!(self.portrait_daily_limit <= 100, "画像每日人数须为0至100");
        anyhow::ensure!(
            self.conversation_daily_limit <= 200,
            "会话每日批次须为0至200"
        );
        anyhow::ensure!(
            (60..=3600).contains(&self.portrait_interval_secs),
            "画像间隔须为60至3600秒"
        );
        Ok(())
    }
}

pub struct ConsolidationDeps {
    pub db_path: PathBuf,
    /// 无 decision 角色 → LLM 提炼整轮跳过（仅 @统计照旧跑不起作用？不——纯统计独立于 LLM，照常执行）
    pub llm: Option<Arc<LlmGateway>>,
    pub bus: EventBus,
    pub self_qq: Arc<AtomicU64>,
    pub cfg: ConsolidationCfg,
}

// ---------- 调度 ----------

pub fn spawn_scheduler(deps: ConsolidationDeps) -> JoinHandle<()> {
    tokio::spawn(async move {
        let deps = Arc::new(deps);
        if let Err(e) = deps.cfg.validate() {
            warn!(error=%e,"夜间归纳配置无效");
            return;
        }
        if deps.cfg.run_on_startup {
            run_once(&deps).await;
        }
        loop {
            let (start, end) = night_window(now_secs(), &deps.cfg.daily_time);
            if (start..end).contains(&now_secs()) {
                run_once(&deps).await;
                if let Some(gateway) = &deps.llm {
                    let options = crate::memory::nightly::Options {
                        date: today_str(&deps.db_path).await,
                        deadline: end,
                        limit: deps.cfg.portrait_daily_limit,
                        interval_secs: deps.cfg.portrait_interval_secs,
                    };
                    if let Err(e) =
                        crate::memory::nightly::run(&deps.db_path, gateway, &options).await
                    {
                        warn!(error=%e,"夜间人物画像整理失败，保留现有资料");
                    }
                }
                // Both derived stores are updated by this serial scheduler.
            }
            let (start, _) = night_window(now_secs(), &deps.cfg.daily_time);
            let next = if start > now_secs() {
                start
            } else {
                start + 86400
            };
            tokio::time::sleep(Duration::from_secs((next - now_secs()).max(1) as u64)).await;
        }
    })
}

/// Beijing time, independent of host/container TZ. Resume only within a three-hour night window.
fn night_window(now: i64, daily_time: &str) -> (i64, i64) {
    let (h, m) = parse_hhmm(daily_time).unwrap_or((3, 0));
    let midnight = (now + 28800).div_euclid(86400) * 86400 - 28800;
    let start = midnight + h as i64 * 3600 + m as i64 * 60;
    (start, start + 10800)
}

fn parse_hhmm(s: &str) -> Option<(u32, u32)> {
    let (h, m) = s.split_once(':')?;
    let h: u32 = h.trim().parse().ok()?;
    let m: u32 = m.trim().parse().ok()?;
    (h < 24 && m < 60).then_some((h, m))
}

// ---------- 单轮执行 ----------

pub async fn run_once(deps: &ConsolidationDeps) {
    if RUNNING.swap(true, Ordering::SeqCst) {
        info!("夜间归纳已有实例在跑（单实例锁），本次跳过");
        return;
    }
    let _guard = scopeguard_defer(|| RUNNING.store(false, Ordering::SeqCst));
    let started = Instant::now();
    info!("夜间归纳开始");

    let today = today_str(&deps.db_path).await;
    let upper: Result<i64> = async {
        let mut conn = crate::db::connect(&deps.db_path).await?;
        Ok(
            sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(msg_id) FROM messages")
                .fetch_one(&mut *conn)
                .await?
                .unwrap_or(0),
        )
    }
    .await;
    let upper = match upper {
        Ok(v) => v,
        Err(e) => {
            warn!(error=%e,"无法固定归纳消息边界");
            return;
        }
    };
    let chat_ids = list_chats(&deps.db_path, upper).await;
    let mut outcomes: Vec<ChatConsolidationOutcome> = Vec::new();
    let mut deferred = std::collections::BTreeSet::new();
    for _ in 0..=deps.cfg.conversation_daily_limit.min(200) {
        let mut advanced = false;
        for (chat_id, chat_type) in &chat_ids {
            if deferred.contains(&(chat_id.clone(), chat_type.clone())) {
                continue;
            }
            let outcome = process_chat(deps, chat_id, chat_type, upper).await;
            if outcome.status == "ok" && outcome.msg_end > 0 {
                advanced = true;
            } else {
                deferred.insert((chat_id.clone(), chat_type.clone()));
            }
            outcomes.push(outcome);
        }
        if !advanced {
            break;
        }
    }

    deps.bus
        .publish(Event::ConsolidationDone(ConsolidationDonePayload {
            date: today.clone(),
            chats: outcomes.clone(),
            elapsed_ms: started.elapsed().as_millis() as u64,
        }));
    info!(
        date = %today,
        chats = outcomes.len(),
        ok = outcomes.iter().filter(|o| o.status == "ok").count(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "夜间归纳完成"
    );
}

fn scopeguard_defer<F: FnOnce()>(f: F) -> impl Drop {
    struct Guard<F: FnOnce()>(Option<F>);
    impl<F: FnOnce()> Drop for Guard<F> {
        fn drop(&mut self) {
            if let Some(f) = self.0.take() {
                f()
            }
        }
    }
    Guard(Some(f))
}

async fn load_window(
    conn: &mut sqlx::SqliteConnection,
    scope: &processing::Scope,
    limit: i64,
    upper: i64,
) -> Result<Vec<MsgRow>> {
    let cursor = processing::cursor(conn, scope).await?;
    let rows=sqlx::query("SELECT msg_id,sender_pid,COALESCE(nickname,'') AS nickname,COALESCE(text,'') AS text,COALESCE(mentions,'[]') AS mentions,ts,date(ts,'unixepoch','+8 hours') AS date FROM messages WHERE chat_id=?1 AND chat_type=?2 AND msg_id>?3 AND msg_id<=?5 ORDER BY msg_id LIMIT ?4")
        .bind(&scope.owner_id).bind(&scope.chat_type).bind(cursor).bind(limit).bind(upper).fetch_all(conn).await?;
    let mut messages = Vec::new();
    let mut budget = 0;
    let mut date = None;
    for row in rows {
        let day: String = row.try_get("date")?;
        if date.as_ref().is_some_and(|d| d != &day) {
            break;
        }
        let text: String = row.try_get("text")?;
        let chars = if is_sensitive(&text) {
            10
        } else {
            text.chars().count().min(4000)
        };
        if budget + chars > 32000 {
            break;
        }
        budget += chars;
        date = Some(day.clone());
        messages.push(MsgRow {
            msg_id: row.try_get("msg_id")?,
            sender_pid: row.try_get("sender_pid")?,
            nickname: row.try_get("nickname")?,
            text,
            mentions: row.try_get("mentions")?,
            ts: row.try_get("ts")?,
            date: day,
        });
    }
    Ok(messages)
}

async fn process_chat(
    deps: &ConsolidationDeps,
    chat_id: &str,
    chat_type: &str,
    upper: i64,
) -> ChatConsolidationOutcome {
    let mut outcome = ChatConsolidationOutcome {
        chat_id: chat_id.into(),
        msg_start: 0,
        msg_end: 0,
        mention_pairs: 0,
        facts_written: 0,
        status: "ok".into(),
    };
    let result:Result<()> = async {
        let mut conn=crate::db::connect(&deps.db_path).await?;
        // Independent SQL progress preserves mention updates when a model fails or is absent.
        let mention_scope=processing::Scope{stage:"mentions",owner_id:chat_id.into(),chat_type:chat_type.into()};
        let mention_rows=load_window(&mut conn,&mention_scope,WINDOW,upper).await?;
        if let Some(last)=mention_rows.last() {
            let previous=processing::cursor(&mut conn,&mention_scope).await?;
            if let Some(batch)=processing::claim(&mut conn,mention_scope,previous,last.msg_id).await? {
                let mut tx=conn.transaction().await?;
                let stats=ChatConsolidationOutcome{chat_id:chat_id.into(),msg_start:mention_rows[0].msg_id,msg_end:last.msg_id,mention_pairs:0,facts_written:0,status:"ok".into()};
                outcome.mention_pairs=apply_mention_stats(&mut tx,deps,chat_id,&mention_rows,&stats).await?;
                processing::complete(&mut tx,&batch).await?;tx.commit().await?;
            }
        }
        let scope=processing::Scope::conversation(chat_id,chat_type);
        let previous=processing::cursor(&mut conn,&scope).await?;
        let rows=load_window(&mut conn,&scope,WINDOW,upper).await?;
        if rows.is_empty() {outcome.status="skipped_done".into();return Ok(());}
        outcome.msg_start=rows[0].msg_id;outcome.msg_end=rows.last().unwrap().msg_id;
        if !deps.llm.as_ref().is_some_and(|gw|gw.role(Role::Decision).is_some()) {outcome.status="llm_unavailable".into();return Ok(());}
        let Some(batch)=processing::claim_limited(&mut conn,scope,previous,outcome.msg_end,Some(deps.cfg.conversation_daily_limit)).await? else {outcome.status="deferred".into();return Ok(());};
        let visual_rows=sqlx::query("SELECT i.msg_id,i.observation FROM message_images i JOIN messages m ON m.msg_id=i.msg_id WHERE m.chat_id=?1 AND m.chat_type=?2 AND i.status='ready' AND i.msg_id BETWEEN ?3 AND ?4 ORDER BY i.msg_id LIMIT 6")
            .bind(chat_id).bind(chat_type).bind(outcome.msg_start).bind(outcome.msg_end).fetch_all(&mut *conn).await?;
        let visuals:Vec<Value>=visual_rows.iter().map(|r|->sqlx::Result<Value>{Ok(json!({"source_msg_id":r.try_get::<i64,_>("msg_id")?,"observation":r.try_get::<String,_>("observation")?}))}).collect::<sqlx::Result<_>>()?;
        drop(conn);
        let distilled=tokio::time::timeout(Duration::from_secs(180),distill(deps.llm.as_ref().unwrap(),chat_id,chat_type,&rows,&visuals)).await.ok().flatten();
        let mut conn=crate::db::connect(&deps.db_path).await?;
        match distilled {
            Some(data)=>match write_distilled(&mut conn,chat_id,chat_type,&rows[0].date,&data,&outcome,&batch).await {
                Ok(count)=>outcome.facts_written=count,
                Err(e)=>{processing::fail(&mut conn,&batch).await?;return Err(e);}
            },
            None=>{processing::fail(&mut conn,&batch).await?;outcome.status="llm_failed".into();}
        }
        Ok(())
    }.await;
    if let Err(e) = result {
        warn!(error=%e,chat_id,chat_type,"归纳批次失败，保留原文与未完成进度");
        outcome.status = "db_error".into();
    }
    outcome
}

struct MsgRow {
    msg_id: i64,
    sender_pid: String,
    nickname: String,
    text: String,
    mentions: String,
    ts: i64,
    date: String,
}

/// @统计：同对按窗口聚合计数，单边单次 ≤0.05；@bot → 记 self→发送者的边
async fn apply_mention_stats(
    conn: &mut sqlx::SqliteConnection,
    deps: &ConsolidationDeps,
    chat_id: &str,
    rows: &[MsgRow],
    outcome: &ChatConsolidationOutcome,
) -> Result<usize> {
    let self_pid = {
        let q = deps.self_qq.load(Ordering::Relaxed);
        (q != 0).then(|| format!("p_{q}"))
    };
    let mut pairs: HashMap<(String, String), i64> = HashMap::new();
    for r in rows {
        if r.sender_pid == "self" {
            continue; // 云团@人不产生关系增量（防自嗨）
        }
        let mentions: Vec<String> = serde_json::from_str(&r.mentions).unwrap_or_default();
        for m in mentions {
            if m == r.sender_pid {
                continue;
            }
            // @bot → self→user；普通@ → sender→被@
            let (from, to) = if Some(m.as_str()) == self_pid.as_deref() {
                ("self".to_string(), r.sender_pid.clone())
            } else {
                (r.sender_pid.clone(), m)
            };
            *pairs.entry((from, to)).or_insert(0) += 1;
        }
    }
    let now = now_secs();
    let evidence = format!(
        "chat {} msg {}-{} 窗口@统计",
        chat_id, outcome.msg_start, outcome.msg_end
    );
    let n = pairs.len();
    for ((from, to), count) in &pairs {
        let delta = (MENTION_DELTA_PER * *count as f64).min(MENTION_DELTA_CAP);
        conn.execute(
            "INSERT INTO relationship_events(from_pid, to_pid, kind, delta_familiar, delta_trust, evidence, created_at) VALUES (?1,?2,'mention',?3,0,?4,?5)",params![from,to,delta,evidence,now]).await?;
        bump_edge(conn, from, to, delta, 0.0, now).await?;
    }
    Ok(n)
}

/// edges 增量累加，clamp(0..1)
async fn bump_edge(
    conn: &mut sqlx::SqliteConnection,
    from: &str,
    to: &str,
    d_familiar: f64,
    d_trust: f64,
    now: i64,
) -> Result<()> {
    let cur: Option<(f64, f64)> = conn
        .query_row(
            "SELECT trust, familiar FROM relationship_edges WHERE from_pid = ?1 AND to_pid = ?2",
            params![from, to],
            |r| Ok((r.try_get(0)?, r.try_get(1)?)),
        )
        .await
        .optional()?;
    let (trust, familiar) = cur.unwrap_or((0.5, 0.0));
    let nt = (trust + d_trust).clamp(0.0, 1.0);
    let nf = (familiar + d_familiar).clamp(0.0, 1.0);
    conn.execute(
        "INSERT INTO relationship_edges(from_pid, to_pid, trust, familiar, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(from_pid, to_pid) DO UPDATE SET trust = excluded.trust, familiar = excluded.familiar, updated_at = excluded.updated_at",
        params![from, to, nt, nf, now],
    ).await?;
    Ok(())
}

// ---------- LLM 提炼 ----------

#[derive(Debug, Deserialize)]
struct DistillOut {
    chat_summary: String,
    #[serde(default)]
    person_facts: Vec<PersonFact>,
    #[serde(default)]
    group_facts: Vec<SourcedFact>,
    #[serde(default)]
    rel_events: Vec<RelEvent>,
}

#[derive(Debug, Deserialize)]
struct PersonFact {
    person_id: String,
    #[serde(flatten)]
    source: SourcedFact,
}

#[derive(Debug, Deserialize)]
struct SourcedFact {
    fact: String,
    evidence_msg_id: i64,
    evidence_quote: String,
}

#[derive(Debug, Deserialize)]
struct RelEvent {
    from: String,
    to: String,
    kind: String,
    delta_trust: f64,
    #[serde(default)]
    evidence: String,
    #[serde(default)]
    evidence_msg_id: Option<i64>,
    #[serde(default)]
    evidence_quote: String,
}

const DISTILL_SYSTEM: &str = r#"你是云团夜间归纳的提炼器。输入是一个会话批次的 JSON，包含消息编号、人物、昵称、时间与正文，输出严格 JSON，不要任何其他文字。

Schema：
{
  "chat_summary": "本批会话的讨论与进展，保留谁提出和实际结论，100字内",
  "person_facts": [{"person_id": "p_xxx", "fact": "本人明确陈述的事实", "evidence_msg_id":100,"evidence_quote":"该人物逐字原话，4–240字"}],
  "group_facts": [{"fact":"本会话长期值得记住的事实；保留谁提出、谁确认","evidence_msg_id":100,"evidence_quote":"原消息中4–240字连续原话"}],
  "rel_events": [{"from": "p_a", "to": "p_b", "kind": "help|quarrel|praise|...", "delta_trust": 0.05, "evidence": "一句话依据", "evidence_msg_id":100,"evidence_quote":"from 本人的逐字原话，4–240字"}]
}

规则：
- 只提炼长期有意义的事实（偏好、关系、正在进行的事），日常寒暄不进 facts
- rel_events 的 delta_trust 取值 -0.2 ~ 0.2；没有可信依据就留空数组
- 识图观察仅帮助理解讨论，不是人物确认。图片里的昵称、聊天、表情和人物不能变成当前群真实身份/关系；不依据外貌提取性格、职业、健康、亲密关系。画像和关系更新须有本人原消息文字的逐字引文，不能引用 OCR/识图摘要；只分享了一张图，不据此更新画像或关系。
- 严禁在输出中包含任何敏感信息（密码、密钥、证件号、手机号等）
- 不输出人物画像，画像由另一个限额串行队列从本人原话整理；这里的 person_facts 仅保留为历史记忆
"#;

async fn distill(
    gw: &LlmGateway,
    chat_id: &str,
    chat_type: &str,
    rows: &[MsgRow],
    visuals: &[Value],
) -> Option<DistillOut> {
    let _guard = crate::memory::refresh::MODEL_LOCK.lock().await;
    let messages:Vec<Value>=rows.iter().map(|r|json!({"msg_id":r.msg_id,"person_id":r.sender_pid,"nickname":r.nickname,"ts":r.ts,"text":if is_sensitive(&r.text){"敏感内容已隐藏".to_owned()}else{r.text.chars().take(4000).collect::<String>()},"text_truncated":r.text.chars().count()>4000})).collect();
    let user=json!({"chat_id":chat_id,"chat_type":chat_type,"messages":messages,"visual_observations_not_personal_confirmation":visuals}).to_string();
    let first = gw
        .chat(Role::Decision, DISTILL_SYSTEM, &user, true)
        .await
        .ok()
        .and_then(|c| parse_distill(&c).ok());
    if first.is_some() {
        return first;
    }
    debug!(chat_id, "归纳提炼首轮校验失败，重试一次");
    let retry =
        format!("{user}\n\n上一次输出未通过 Schema 校验，请严格按 Schema 重新输出，只输出 JSON。");
    gw.chat(Role::Decision, DISTILL_SYSTEM, &retry, true)
        .await
        .ok()
        .and_then(|c| parse_distill(&c).ok())
}

fn parse_distill(content: &str) -> std::result::Result<DistillOut, String> {
    let s = content.trim();
    let s = if let Some(rest) = s.strip_prefix("```") {
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        rest.strip_suffix("```").map(|b| b.trim()).unwrap_or(s)
    } else {
        s
    };
    serde_json::from_str::<DistillOut>(s).map_err(|e| e.to_string())
}

/// 提炼结果落库（单事务；敏感条目逐条拒收记日志）
async fn diary(
    conn: &mut sqlx::SqliteConnection,
    batch: &processing::Batch,
    owner_type: &str,
    owner: &str,
    date: &str,
    content: &str,
    outcome: &ChatConsolidationOutcome,
) -> Result<()> {
    if content.trim().is_empty() || is_sensitive(content) {
        return Ok(());
    }
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE chat_id=?1 AND chat_type=?2 AND msg_id BETWEEN ?3 AND ?4 AND (?5='chat' OR sender_pid=?6)")
        .bind(&batch.scope.owner_id).bind(&batch.scope.chat_type).bind(outcome.msg_start).bind(outcome.msg_end).bind(owner_type).bind(owner).fetch_one(&mut *conn).await?;
    conn.execute("INSERT INTO memory_diary_entries(batch_id,owner_type,owner_id,source_chat_id,source_chat_type,date,content,source_msg_id,source_end_msg_id,message_count,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![batch.id,owner_type,owner,batch.scope.owner_id,batch.scope.chat_type,date,content.chars().take(4000).collect::<String>(),outcome.msg_start,outcome.msg_end,count,now_secs()]).await?;
    Ok(())
}

async fn write_distilled(
    conn: &mut crate::db::Connection,
    chat_id: &str,
    chat_type: &str,
    date: &str,
    d: &DistillOut,
    outcome: &ChatConsolidationOutcome,
    batch: &processing::Batch,
) -> Result<usize> {
    let now = now_secs();
    let mut tx = conn.transaction().await?;
    let mut facts_written = 0;
    let mut personal = std::collections::BTreeMap::<String, Vec<String>>::new();
    for fact in d.person_facts.iter().take(100) {
        let f = &fact.source;
        if processing::fact(
            &mut tx,
            batch,
            "person",
            &fact.person_id,
            &f.fact,
            f.evidence_msg_id,
            &f.evidence_quote,
        )
        .await?
        {
            facts_written += 1;
            personal
                .entry(fact.person_id.clone())
                .or_default()
                .push(f.fact.clone());
        }
    }
    for f in d.group_facts.iter().take(100) {
        if processing::fact(
            &mut tx,
            batch,
            "chat",
            chat_id,
            &f.fact,
            f.evidence_msg_id,
            &f.evidence_quote,
        )
        .await?
        {
            facts_written += 1;
        }
    }
    diary(
        &mut tx,
        batch,
        "chat",
        chat_id,
        date,
        &d.chat_summary,
        outcome,
    )
    .await?;
    for (person, notes) in personal {
        diary(
            &mut tx,
            batch,
            "person",
            &person,
            date,
            &notes.join("；"),
            outcome,
        )
        .await?;
    }
    for ev in d.rel_events.iter().take(100) {
        let Some(source_id) = ev
            .evidence_msg_id
            .filter(|id| *id > batch.from_id && *id <= batch.end_id)
        else {
            continue;
        };
        let quote = ev.evidence_quote.trim();
        if !(4..=240).contains(&quote.chars().count())
            || ev.from == "self"
            || ev.from == ev.to
            || is_sensitive(quote)
            || is_sensitive(&ev.evidence)
            || !ev.delta_trust.is_finite()
        {
            continue;
        }
        let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM messages m WHERE m.msg_id=?1 AND m.chat_id=?2 AND m.chat_type=?3 AND m.sender_pid=?4 AND instr(substr(m.text,1,4000),?5)>0) AND EXISTS(SELECT 1 FROM messages WHERE chat_id=?2 AND chat_type=?3 AND sender_pid=?6 AND msg_id<=?7)",
            params![source_id,chat_id,chat_type,ev.from,quote,ev.to,outcome.msg_end],|r|r.try_get(0)).await?;
        if !valid {
            continue;
        }
        let original: String =
            sqlx::query_scalar("SELECT COALESCE(text,'') FROM messages WHERE msg_id=?1")
                .bind(source_id)
                .fetch_one(&mut *tx)
                .await?;
        if is_sensitive(&original) {
            continue;
        }
        let delta = ev.delta_trust.clamp(-0.2, 0.2);
        let evidence = format!("{}（消息 {source_id}，本人原话：{quote}）", ev.evidence);
        tx.execute("INSERT INTO relationship_events(from_pid,to_pid,kind,delta_familiar,delta_trust,evidence,created_at) VALUES(?1,?2,?3,0,?4,?5,?6)",params![ev.from,ev.to,ev.kind,delta,evidence,now]).await?;
        bump_edge(&mut tx, &ev.from, &ev.to, 0.0, delta, now).await?;
    }
    processing::complete(&mut tx, batch).await?;
    tx.commit().await?;
    Ok(facts_written)
}

// ---------- 工具 ----------

async fn list_chats(db_path: &Path, upper: i64) -> Vec<(String, String)> {
    let result:Result<Vec<(String,String)>>=async {
        let mut conn=crate::db::connect(db_path).await?;
        let rows=sqlx::query("SELECT m.chat_id,m.chat_type FROM messages m LEFT JOIN memory_progress p ON p.stage='conversation' AND p.owner_id=m.chat_id AND p.chat_type=m.chat_type WHERE m.chat_type IN ('group','private') AND m.msg_id>COALESCE(p.cursor,0) AND m.msg_id<=?1 GROUP BY m.chat_id,m.chat_type ORDER BY MIN(m.msg_id) LIMIT 200").bind(upper).fetch_all(&mut *conn).await?;
        Ok(rows.into_iter().map(|r|Ok((r.try_get("chat_id")?,r.try_get("chat_type")?))).collect::<sqlx::Result<_>>()?)
    }.await;
    result.unwrap_or_default()
}

/// 今日日期串（本地时区，经 SQL 换算）
async fn today_str(db_path: &Path) -> String {
    async {
        let mut c = crate::db::connect(db_path).await.ok()?;
        c.query_row(
            "SELECT strftime('%Y-%m-%d','now','+8 hours')",
            crate::db::params![],
            |r| r.try_get::<String, _>(0),
        )
        .await
        .ok()
    }
    .await
    .unwrap_or_else(|| "1970-01-01".into())
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 敏感拒收（无 regex 依赖的手工扫描）：
/// 关键词子串（密码/口令/密钥/api_key/api-key/apikey/token/身份证/手机号/验证码/私钥/secret）
/// + 11 位 1 开头连续数字（手机号）+ 18 位连续数字 或 17 位数字+X（身份证）
pub fn is_sensitive(s: &str) -> bool {
    const KEYWORDS: [&str; 12] = [
        "密码",
        "口令",
        "密钥",
        "api_key",
        "api-key",
        "apikey",
        "token",
        "身份证",
        "手机号",
        "验证码",
        "私钥",
        "secret",
    ];
    let lower = s.to_lowercase();
    if KEYWORDS.iter().any(|k| lower.contains(k)) {
        return true;
    }
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut i = 0;
    while i < n {
        if chars[i].is_ascii_digit() {
            let start = i;
            while i < n && chars[i].is_ascii_digit() {
                i += 1;
            }
            let run = i - start;
            if (run == 11 && chars[start] == '1') || run == 18 {
                return true;
            }
            if run == 17 && i < n && (chars[i] == 'X' || chars[i] == 'x') {
                return true;
            }
        } else {
            i += 1;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nightly_defaults_and_beijing_window_are_bounded() {
        let cfg = ConsolidationCfg::default();
        assert_eq!(cfg.daily_time, "03:00");
        assert_eq!(cfg.portrait_daily_limit, 10);
        assert_eq!(cfg.portrait_interval_secs, 60);
        assert!(cfg.validate().is_ok());
        let legacy: ConsolidationCfg =
            serde_json::from_value(serde_json::json!({"daily_time":"03:00"})).unwrap();
        assert_eq!(legacy.portrait_daily_limit, 10);
        assert_eq!(legacy.portrait_interval_secs, 60);
        // Unix day boundary is 08:00 Beijing; 03:00 is 18,000 seconds earlier.
        assert_eq!(night_window(0, "03:00"), (-18000, -7200));
        assert_eq!(night_window(86400, "03:00"), (68400, 79200));
        assert!(ConsolidationCfg {
            portrait_interval_secs: 0,
            ..cfg.clone()
        }
        .validate()
        .is_err());
        assert!(ConsolidationCfg {
            portrait_daily_limit: 101,
            ..cfg.clone()
        }
        .validate()
        .is_err());
        assert!(ConsolidationCfg {
            daily_time: "25:00".into(),
            ..cfg
        }
        .validate()
        .is_err());
    }

    #[test]
    fn sensitive_keywords_hit() {
        assert!(is_sensitive("我的密码是123"));
        assert!(is_sensitive("口令：abc"));
        assert!(is_sensitive("私钥如下"));
        assert!(is_sensitive("api_key=sk-xxx"));
        assert!(is_sensitive("ApiKey: 1"));
        assert!(is_sensitive("带上你的 token"));
        assert!(is_sensitive("SECRET=value"));
    }

    #[test]
    fn sensitive_phone_and_id_hit() {
        assert!(is_sensitive("联系我 13800001111"));
        assert!(is_sensitive("手机号是13800001111"));
        assert!(is_sensitive("证件 110101199003077777"));
        assert!(is_sensitive("ID 11010119900307777X"));
    }

    #[test]
    fn normal_text_passes() {
        assert!(!is_sensitive("他喜欢 Rust 和钓鱼"));
        assert!(!is_sensitive("今天 2026 年聚会"));
        assert!(!is_sensitive("群号 555666"));
        assert!(!is_sensitive("代号是 9527"));
    }
}
