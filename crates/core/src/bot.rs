//! Bot 系统：聊天管线——订阅 MessageReceived → Prefilter →（放行）→ 成本闸（llm.chat 内）→
//! 组上下文 → Decision → 副作用 + 事件；action=reply 再接 bot_chat → Bubbleizer → 发送队列；
//! action=send_meme → 抽图 → 发送队列。
//! 铁律：core 不依赖具体协议端；self_qq 以共享原子单元注入，发送以 SendFn 闭包注入。
//! 热应用（组件级）：llm / prefilter 走共享 RwLock 槽位，每条消息读取当前值，
//! WebUI config 写回后换槽即生效（其余组件 TODO，见 WebUI config_api）。

use crate::db::SqliteExt;
use crate::decision::{self, DecisionAction};
use crate::event::{BubbleSentPayload, Event, EventBus, MessageReceivedPayload};
use crate::llm::{LlmGateway, Role};
use crate::meme;
use crate::prefilter::{SelfMsgIds, Verdict};
use crate::reply_engine::{self, EngineHandle};
use crate::state::MoodState;
use sqlx::Row;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

/// LLM 共享槽（配置热应用：WebUI 写回 providers.toml 后整体换槽）
pub type SharedLlm = Arc<RwLock<Option<Arc<LlmGateway>>>>;
/// Prefilter 阈值共享槽
pub type SharedPrefilter = Arc<RwLock<crate::prefilter::Config>>;

#[derive(Clone)]
pub struct PipelineDeps {
    pub bus: EventBus,
    pub db_path: PathBuf,
    pub llm: SharedLlm,
    pub self_qq: Arc<AtomicU64>,
    pub self_ids: SelfMsgIds,
    pub mood: MoodState,
    pub prefilter: SharedPrefilter,
    /// 回复引擎；None 时 reply/send_meme 只记事件跳过（如 adapter 未接入）
    pub reply: Option<EngineHandle>,
    /// 回复形态参数热应用槽（bubbleize 泡顶/单泡上限在入队时读取）
    pub reply_cfg: crate::reply_engine::SharedReplyCfg,
    /// 上下文预算热应用槽（组 Bot Context 每消息读取）
    pub ctx_cfg: crate::context_builder::SharedContextCfg,
    /// data/memes 根目录（meme 抽图）
    pub memes_dir: PathBuf,
    /// Skill 注册表（Q-S02 Decision 自-driven 调用 Skill）；None 时 Decision 不提供 invoke_skill 选项
    pub skill_registry: Option<crate::skills::SkillRegistry>,
    /// Q52 per-chat mpsc 容量热应用槽（G008）；新 worker spawn 时读
    pub per_chat_cap: SharedPerChatCap,
}

/// Q52：按 chat 隔离的 dispatcher + worker 拓扑——
/// dispatcher 订阅 broadcast，按 chat_id 派发到 per-chat mpsc；
/// 每 chat 一个 worker 协程串行 handle（保序）；不同 chat 并发，互不堵塞。
/// mpsc 容量来自共享槽（[pipeline].per_chat_queue_cap，默认 32，G008 热应用）；
/// 单 chat 瞬时洪峰超过则降级为同步 handle（保底不丢，仅阻塞 dispatcher 一拍）。
/// 热应用语义：改槽不影响已 spawn 的 worker（容量在 spawn 时定型），只影响新 worker。
pub type SharedPerChatCap = Arc<RwLock<usize>>;

/// Q55：扫描 messages 表中 processed_at IS NULL 的非 self 消息，按 ts 升序回放给 handle。
/// 在 spawn_pipeline 之前调用；返回回放条数。
/// 跳过 /image 命令——media 副作用大且用户当时已得到响应（避免重启刷图）。
/// 只回放最近 REPLAY_WINDOW_SECS 内的未处理；更早的直接标记已处理（防历史脏数据雪崩）。
const REPLAY_WINDOW_SECS: i64 = 3600;

pub async fn replay_pending(deps: &PipelineDeps) -> anyhow::Result<usize> {
    use crate::db::params;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let cutoff = now - REPLAY_WINDOW_SECS;
    let mut conn = crate::db::connect(&deps.db_path).await?;
    // 先把窗口外的未处理全部标记放弃（战的，不回放）
    let abandoned = conn
        .execute(
            "UPDATE messages SET processed_at = ?1
         WHERE processed_at IS NULL AND sender_pid != 'self' AND ts < ?2",
            params![now, cutoff],
        )
        .await?;
    if abandoned > 0 {
        info!(
            count = abandoned,
            cutoff_secs = REPLAY_WINDOW_SECS,
            "Q55 放弃窗口外未处理消息"
        );
    }
    let mut stmt = conn
        .prepare(
            "SELECT msg_id, chat_id, chat_type, sender_pid, text, at_me, has_image, reply_to, ts
         FROM messages
         WHERE processed_at IS NULL AND sender_pid != 'self' AND ts >= ?1
         ORDER BY ts ASC, msg_id ASC",
        )
        .await?;
    let rows = stmt
        .query_map(params![cutoff], |r| {
            Ok((
                r.try_get::<i64, _>(0)?,
                r.try_get::<String, _>(1)?,
                r.try_get::<String, _>(2)?,
                r.try_get::<String, _>(3)?,
                r.try_get::<Option<String>, _>(4)?,
                r.try_get::<i64, _>(5)?,
                r.try_get::<i64, _>(6)?,
                r.try_get::<Option<i64>, _>(7)?,
                r.try_get::<i64, _>(8)?,
            ))
        })
        .await?;
    drop(stmt);
    drop(conn); // Rows are owned; replay must not reserve a pool slot while waiting for models.
    let mut count = 0usize;
    for row in rows {
        let (msg_id, chat_id, chat_type, sender_pid, text, at_me, has_image, reply_to, ts) =
            match row {
                Ok(r) => r,
                Err(e) => {
                    debug!(error = %e, "回放读取行失败，跳过");
                    continue;
                }
            };
        let text = text.unwrap_or_default();
        // Q55 裁决：回放跳过生图命令，避免重启后重复调用付费 API。
        if is_image_command(&text) {
            mark_processed(&deps.db_path, msg_id).await;
            debug!(msg_id, "回放跳过 /image 命令（直接标记已处理）");
            continue;
        }
        let payload = MessageReceivedPayload {
            msg_id,
            chat_id,
            chat_type,
            sender_pid,
            text,
            at_me: at_me != 0,
            has_image: has_image != 0,
            reply_to,
            sender_bot: false, // 已落库的非 self 消息不再重判 R2（回放即信任当时 ingest 判定）
            image_urls: Vec::new(), // 不回放偷图流程（image_urls 不落 messages）
            ts,
        };
        handle(deps, &payload).await;
        count += 1;
    }
    if count > 0 {
        info!(count, "Q55 恢复消费完成");
    }
    Ok(count)
}

pub fn spawn_pipeline(deps: PipelineDeps) -> JoinHandle<()> {
    type ChatLanes =
        std::collections::HashMap<String, tokio::sync::mpsc::Sender<(MessageReceivedPayload, i64)>>;
    tokio::spawn(async move {
        let mut rx = deps.bus.subscribe();
        if !decision_ready(&deps.llm) {
            warn!("Decision 角色未配置（providers.toml）：配置前消息过 Prefilter 后直接 ignore，管线照常运行");
        }
        info!("Decision 管线已启动（Q52 per-chat 并发 + Q54 10s 窗口聚合）");
        let deps = Arc::new(deps);
        let mut lanes = ChatLanes::new();
        // 取消 dispatcher 时同时取消其 worker，防止停机后继续生成/入队。
        let mut workers = tokio::task::JoinSet::new();

        // Q54 窗口聚合器:
        //   固定 10s 窗口,同 chat 普通消息追加,不重复开窗;anchor 固定为第一条;
        //   窗口到期 fire → anchor 进 per-chat worker 触发一次 decide;
        //   以 batch 最后一个消息 ID 固定本轮上界；worker 开始处理时读取一次快照，
        //   Decision 与 bot_chat 共享，排队或模型等待期间的新消息不混入本轮。
        //   others 需立刻 mark_processed,Q55 重启不再回放(窗口 fire 后 anchor 替身已代表整个 batch)。
        let deps_for_fire = Arc::clone(&deps);
        let lanes_for_fire: Arc<RwLock<ChatLanes>> = Arc::new(RwLock::new(ChatLanes::new()));
        let lanes_for_fire_clone = Arc::clone(&lanes_for_fire);
        let on_fire: crate::window::OnFire = Arc::new(move |batch: crate::window::WindowBatch| {
            let deps = Arc::clone(&deps_for_fire);
            let lanes_clone = Arc::clone(&lanes_for_fire_clone);
            Box::pin(async move {
                let cutoff = batch
                    .others
                    .iter()
                    .map(|m| m.msg_id)
                    .fold(batch.anchor.msg_id, i64::max);
                // others 立刻回写 processed_at(否则 Q55 重启会把它们当未处理回放,雪崩)
                for om in &batch.others {
                    mark_processed(&deps.db_path, om.msg_id).await;
                }

                // anchor 进 per-chat worker(保序不丢);worker 拿不到 lane 则同步保底
                let chat_id = batch.anchor.chat_id.clone();
                let tx_opt = lanes_clone.read().unwrap().get(&chat_id).cloned();
                match tx_opt {
                    Some(tx) => {
                        if let Err(e) = tx.try_send((batch.anchor.clone(), cutoff)) {
                            debug!(chat_id = %chat_id, error = %e, "Q54 anchor 入队失败,同步保底");
                            handle_at(&deps, &batch.anchor, cutoff).await;
                        }
                    }
                    None => {
                        debug!(chat_id = %chat_id, "Q54 anchor 找不到 lane(不该发生),同步保底");
                        handle_at(&deps, &batch.anchor, cutoff).await;
                    }
                }
            })
        });
        let mut aggregator =
            crate::window::WindowAggregator::new(crate::window::DEFAULT_WINDOW_SECS);

        loop {
            match rx.recv().await {
                Ok(Event::MessageReceived(m)) => {
                    // 生图命令走 dispatcher→worker 直派，不进消息聚合窗口。
                    let is_media_cmd = is_image_command(&m.text);

                    if is_media_cmd {
                        let lane = lanes.entry(m.chat_id.clone()).or_insert_with(|| {
                            let cap = *deps.per_chat_cap.read().unwrap();
                            let (tx, mut lane_rx) = tokio::sync::mpsc::channel::<(
                                MessageReceivedPayload,
                                i64,
                            )>(cap.max(1));
                            let deps2 = Arc::clone(&deps);
                            let chat = m.chat_id.clone();
                            workers.spawn(async move {
                                while let Some((mm, cutoff)) = lane_rx.recv().await {
                                    handle_at(&deps2, &mm, cutoff).await;
                                }
                                debug!(chat_id = %chat, "chat worker 退出(dispatcher 关闭)");
                            });
                            tx
                        });
                        lanes_for_fire
                            .write()
                            .unwrap()
                            .insert(m.chat_id.clone(), lane.clone());
                        if let Err(e) = lane.try_send((m.clone(), m.msg_id)) {
                            debug!(chat_id = %m.chat_id, error = %e, "per-chat 队列满,同步处理保底");
                            handle(&deps, &m).await;
                        }
                        continue;
                    }

                    // 非命令路径:对该 chat 首条建立 lane 并 record map
                    let lane = lanes.entry(m.chat_id.clone()).or_insert_with(|| {
                        let cap = *deps.per_chat_cap.read().unwrap();
                        let (tx, mut lane_rx) =
                            tokio::sync::mpsc::channel::<(MessageReceivedPayload, i64)>(cap.max(1));
                        let deps2 = Arc::clone(&deps);
                        let chat = m.chat_id.clone();
                        workers.spawn(async move {
                            while let Some((mm, cutoff)) = lane_rx.recv().await {
                                handle_at(&deps2, &mm, cutoff).await;
                            }
                            debug!(chat_id = %chat, "chat worker 退出(dispatcher 关闭)");
                        });
                        tx
                    });
                    lanes_for_fire
                        .write()
                        .unwrap()
                        .insert(m.chat_id.clone(), lane.clone());

                    // 进 Q54 聚合器,窗口到期由 on_fire 把 anchor 推给 worker(见上)
                    aggregator.feed(m, on_fire.clone());
                }
                Ok(_) => {} // 其余事件类型本管线不消费
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!(skipped = n, "管线消费滞后，跳过旧事件");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    info!("管线退出：事件总线已关闭");
                    break;
                }
            }
        }
    })
}

fn decision_ready(llm: &SharedLlm) -> bool {
    llm.read()
        .unwrap()
        .as_ref()
        .map(|g| g.role(Role::Decision).is_some())
        .unwrap_or(false)
}

/// reply_engine::route_of 的转发（chat_type/target 解析）
fn reply_engine_route(m: &MessageReceivedPayload) -> anyhow::Result<(reply_engine::ChatType, u64)> {
    crate::reply_engine::route_of(m).map(|(ct, target, _)| (ct, target))
}

fn is_image_command(text: &str) -> bool {
    let text = text.trim();
    ["/image", "/画"].iter().any(|command| {
        text == *command
            || text
                .strip_prefix(command)
                .is_some_and(|rest| rest.starts_with(char::is_whitespace))
    })
}

fn enqueue_image_plugin_disabled(deps: &PipelineDeps, m: &MessageReceivedPayload) {
    let Some(engine) = &deps.reply else { return };
    let Ok((chat_type, target)) = reply_engine_route(m) else {
        return;
    };
    engine.enqueue(reply_engine::ReplyJob {
        chat_id: m.chat_id.clone(),
        chat_type,
        target,
        anchor_msg_id: m.msg_id,
        mention: false,
        mention_qq: None,
        kind: reply_engine::JobKind::Bubbles(vec![reply_engine::Bubble {
            text: "NAI 生图插件尚未启用，请在插件面板启用并配置 API Key。".into(),
            at: false,
            meme: None,
        }]),
    });
}

/// Q55 包装层：处理完毕无条件回写 messages.processed_at（含 Prefilter Drop 分支——
/// 「处理过」不等于「回复过」；重启回放只扫进程崩溃前来不及进 handle 的消息）。
async fn handle(deps: &PipelineDeps, m: &MessageReceivedPayload) {
    handle_at(deps, m, m.msg_id).await;
}

async fn handle_at(deps: &PipelineDeps, m: &MessageReceivedPayload, cutoff: i64) {
    handle_inner(deps, m, cutoff).await;
    mark_processed(&deps.db_path, m.msg_id).await;
}

async fn mark_processed(db_path: &std::path::Path, msg_id: i64) {
    if msg_id <= 0 {
        return; // 回放路径构造的合成消息没有正 msg_id
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let res = async {
        let mut c = crate::db::connect(db_path).await?;
        c.execute(
            "UPDATE messages SET processed_at = ?1 WHERE msg_id = ?2",
            crate::db::params![now, msg_id],
        )
        .await
        .map_err(anyhow::Error::from)
    }
    .await;
    if let Err(e) = res {
        debug!(msg_id, error = %e, "processed_at 回写失败（下轮重启会回放该条，幂等）");
    }
}

async fn handle_inner(deps: &PipelineDeps, m: &MessageReceivedPayload, cutoff: i64) {
    if handle_plugin_command(deps, m).await {
        return;
    }
    // 生图只由已启用的 NAI 插件处理；不再回退到旧 Core 媒体实现。
    if is_image_command(&m.text) {
        enqueue_image_plugin_disabled(deps, m);
        return;
    }

    let q = deps.self_qq.load(Ordering::Relaxed);
    let self_pid = (q != 0).then(|| format!("p_{q}"));
    let pf = *deps.prefilter.read().unwrap(); // 热应用：每条取当前阈值
    match crate::prefilter::check(&pf, self_pid.as_deref(), &deps.self_ids, &deps.db_path, m).await
    {
        Verdict::Drop(rule) => debug!(rule, chat_id = %m.chat_id, "Prefilter 丢弃"),
        Verdict::Pass => {
            let gw = match deps.llm.read().unwrap().clone() {
                Some(g) if g.role(Role::Decision).is_some() => g,
                _ => {
                    debug!(chat_id = %m.chat_id, "Decision 角色不可用，本条 ignore");
                    return;
                }
            };
            // 成本闸热应用：prefilter 槽 → gateway（每条对齐一次，换槽/调参即生效）
            gw.set_cost_per_min(pf.decision_cost_per_min.max(1) as usize);
            let ctx_cfg = *deps.ctx_cfg.read().unwrap();
            let snapshot = match crate::context_builder::capture_reply_snapshot(
                &deps.db_path,
                m,
                cutoff,
                &ctx_cfg,
            )
            .await
            {
                Ok(snapshot) => snapshot,
                result => {
                    warn!(error=?result,"无法读取回复上下文，跳过本轮");
                    return;
                }
            };
            let outcome = decision::decide(
                &deps.db_path,
                &gw,
                &deps.bus,
                &deps.mood,
                m,
                deps.skill_registry.as_ref(),
                &snapshot,
            )
            .await;

            // action=invoke_skill：Skill 执行（Q-S02）。Skill 内部默认渲染模板；
            // 插件可以 override invoke 自己调 chat LLM。Skill 输出经 bot_chat Bubbleizer 发送。
            if outcome.output.action == DecisionAction::InvokeSkill {
                let skill_name = outcome.output.skill_name.clone().unwrap_or_default();
                let reg = match deps.skill_registry.as_ref() {
                    Some(r) => r,
                    None => {
                        warn!(chat_id = %m.chat_id, skill = %skill_name, "SkillRegistry 未注入，invoke_skill 跳过");
                        return;
                    }
                };
                let skill = match reg.get(&skill_name) {
                    Some(s) => s,
                    None => {
                        warn!(chat_id = %m.chat_id, skill = %skill_name, "invoke_skill 所选 Skill 未注册（decision 已过滤，理论上到不了这）");
                        return;
                    }
                };
                let engine = match deps.reply.as_ref() {
                    Some(e) => e,
                    None => {
                        debug!(chat_id = %m.chat_id, "回复引擎未接入，invoke_skill 跳过");
                        return;
                    }
                };
                let (chat_type, target) = match reply_engine_route(m) {
                    Ok(r) => r,
                    Err(e) => {
                        warn!(error = %e, chat_id = %m.chat_id, "invoke_skill 路由失败");
                        return;
                    }
                };
                let ctx = crate::tools::ToolCtx {
                    task_id: None,
                    chat_id: m.chat_id.clone(),
                    chat_type: m.chat_type.clone(),
                    sender_pid: m.sender_pid.clone(),
                    locale: Some("zh-CN".into()),
                };
                let mut slots = outcome
                    .output
                    .skill_slots
                    .clone()
                    .unwrap_or_else(|| serde_json::json!({}));
                // 把消息文本保底塞进 user_text 槽（Skill 模板最常用槽位）
                if slots.get("user_text").is_none() {
                    slots["user_text"] = serde_json::Value::String(m.text.clone());
                }
                // 把 sender 名字等拼进 context 槽
                if slots.get("context").is_none() {
                    slots["context"] = serde_json::json!(format!(
                        "chat_id={} sender={} chat_type={}",
                        m.chat_id, m.sender_pid, m.chat_type
                    ));
                }
                add_message_slots(deps, m, &mut slots).await;
                match skill.invoke(&ctx, slots).await {
                    Ok(out) => {
                        info!(
                            chat_id = %m.chat_id,
                            skill = %skill_name,
                            summary = %out.summary,
                            "invoke_skill 执行完成"
                        );
                        // Skill 的 rendered 内容按普通回复发出（走 bot_chat 再走 Bubbleizer）
                        // Phase 2 简化：直接发送 out.data.rendered，不再让 bot_chat 二次包装；
                        // RoleHint 与 persona 化后续由 Bot Chat 自身处理
                        let rendered = out
                            .data
                            .get("rendered")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| out.summary.clone());
                        // Skill 输出包成单 Bubble（无 at/无 meme），走 Bubbles 路径享受打字延时与作废核对
                        engine.enqueue(reply_engine::ReplyJob {
                            chat_id: m.chat_id.clone(),
                            chat_type,
                            target,
                            anchor_msg_id: m.msg_id,
                            mention: false,
                            mention_qq: None,
                            kind: reply_engine::JobKind::Bubbles(vec![reply_engine::Bubble {
                                text: rendered,
                                at: false,
                                meme: None,
                            }]),
                        });
                        for path in out.artifacts {
                            if let Err(e) = engine.enqueue_attachment_with_retention(
                                m.chat_id.clone(),
                                chat_type,
                                target,
                                path.into(),
                                skill.artifact_retention(),
                            ) {
                                warn!(error=%e, "插件附件入队失败");
                            }
                        }
                    }
                    Err(e) => {
                        warn!(chat_id = %m.chat_id, skill = %skill_name, error = %e, "invoke_skill 执行失败");
                        engine.enqueue(reply_engine::ReplyJob {
                            chat_id: m.chat_id.clone(),
                            chat_type,
                            target,
                            anchor_msg_id: m.msg_id,
                            mention: false,
                            mention_qq: None,
                            kind: reply_engine::JobKind::Bubbles(vec![reply_engine::Bubble {
                                text: format!(
                                    "处理失败：{}",
                                    e.to_string().chars().take(500).collect::<String>()
                                ),
                                at: false,
                                meme: None,
                            }]),
                        });
                    }
                }
                return;
            }
            if outcome.output.action == DecisionAction::SendMeme {
                // send_meme：抽图→发送队列（直接发图，与普通回复同队列保序）；库空记事件不吵
                let cat = outcome
                    .output
                    .meme_type
                    .clone()
                    .unwrap_or_else(|| "misc".into());
                match deps.reply.as_ref() {
                    Some(engine) => match reply_engine_route(m) {
                        Ok((chat_type, target)) => {
                            let picked = match meme::pick(&deps.db_path, &deps.memes_dir, &cat)
                                .await
                            {
                                some @ Some(_) => some,
                                None => meme::pick(&deps.db_path, &deps.memes_dir, "misc").await,
                            };
                            match picked {
                                Some(path) => {
                                    info!(chat_id = %m.chat_id, category = %cat, "send_meme 抽图入队");
                                    engine.enqueue_image(
                                        m.chat_id.clone(),
                                        chat_type,
                                        target,
                                        path,
                                    );
                                }
                                None => {
                                    debug!(chat_id = %m.chat_id, category = %cat, "meme 库空，跳过发图");
                                    deps.bus.publish(Event::BubbleSent(BubbleSentPayload {
                                        chat_id: m.chat_id.clone(),
                                        bubble_index: 0,
                                        total: 0,
                                        ok: false,
                                        note: Some(format!("meme 库空（类别 {cat}）")),
                                    }));
                                }
                            }
                        }
                        Err(e) => warn!(error = %e, "send_meme 路由失败"),
                    },
                    None => debug!(chat_id = %m.chat_id, "回复引擎未接入，send_meme 跳过"),
                }
                return;
            }
            // action=start_task：建 tasks 行 + TaskCreated 事件（agent::create_task 落两笔）；
            // spawn_runner 订阅事件后立即起执行协程。静默：不发群消息（Q-A03）
            if outcome.output.action == DecisionAction::StartTask {
                let goal = outcome
                    .output
                    .task_goal
                    .clone()
                    .unwrap_or_else(|| m.text.chars().take(120).collect());
                if let Err(e) = crate::agent::create_task(
                    &deps.db_path,
                    &deps.bus,
                    &m.chat_id,
                    &m.sender_pid,
                    &goal,
                )
                .await
                {
                    warn!(chat_id = %m.chat_id, error = %e, "start_task 落地失败");
                } else {
                    info!(chat_id = %m.chat_id, goal = %goal, "start_task 已建仓");
                }
                return;
            }
            if outcome.output.action != DecisionAction::Reply {
                return; // ignore 仅由 DecisionMade 事件记录
            }
            // action=reply → bot_chat → Bubbleizer → 发送队列
            let bot_chat_ready = gw.role(Role::BotChat).is_some();
            match (&deps.reply, bot_chat_ready) {
                (Some(engine), true) => {
                    let reply_cfg = *deps.reply_cfg.read().unwrap();
                    if let Err(e) = reply_engine::prepare_and_enqueue(
                        engine,
                        &snapshot,
                        &gw,
                        &deps.mood,
                        m,
                        &outcome.output,
                        &ctx_cfg,
                        &reply_cfg,
                    )
                    .await
                    {
                        warn!(chat_id = %m.chat_id, error = %e, "bot_chat 回复准备失败，跳过");
                        deps.bus.publish(Event::BubbleSent(BubbleSentPayload {
                            chat_id: m.chat_id.clone(),
                            bubble_index: 0,
                            total: 0,
                            ok: false,
                            note: Some(format!("bot_chat 调用失败: {e}")),
                        }));
                    }
                }
                _ => {
                    debug!(chat_id = %m.chat_id, "bot_chat 角色未配置或回复引擎未接入，reply 跳过");
                    deps.bus.publish(Event::BubbleSent(BubbleSentPayload {
                        chat_id: m.chat_id.clone(),
                        bubble_index: 0,
                        total: 0,
                        ok: false,
                        note: Some("bot_chat 角色未配置或回复引擎未接入，跳过回复".into()),
                    }));
                }
            }
        }
    }
}

/// Explicit plugin commands use registered, permission-checked skills even without LLM roles.
async fn add_message_slots(
    deps: &PipelineDeps,
    m: &MessageReceivedPayload,
    slots: &mut serde_json::Value,
) {
    slots["reply_to"] = serde_json::json!(m.reply_to);
    let mentions = async {
        let mut conn = crate::db::connect(&deps.db_path).await.ok()?;
        let mentions: String = conn
            .query_row(
                "SELECT mentions FROM messages WHERE msg_id=?1 AND chat_id=?2 AND chat_type=?3",
                crate::db::params![m.msg_id, m.chat_id, m.chat_type],
                |r| r.try_get(0),
            )
            .await
            .ok()?;
        serde_json::from_str::<Vec<String>>(&mentions).ok()
    }
    .await
    .unwrap_or_default();
    let own_qq = deps.self_qq.load(Ordering::Relaxed);
    let mut targets: Vec<u64> = mentions
        .iter()
        .filter_map(|p| p.strip_prefix("p_")?.parse().ok())
        .filter(|n| *n != own_qq)
        .collect();
    targets.sort_unstable();
    targets.dedup();
    slots["mentioned_qqs"] = serde_json::json!(targets);
}

async fn handle_plugin_command(deps: &PipelineDeps, m: &MessageReceivedPayload) -> bool {
    let own_qq = deps.self_qq.load(Ordering::Relaxed);
    if m.sender_pid == "self" || (own_qq != 0 && m.sender_pid == format!("p_{own_qq}")) {
        return false;
    }
    let Some(registry) = deps.skill_registry.as_ref() else {
        return false;
    };
    let command = match registry.command(&m.text) {
        Ok(Some(command)) => command,
        Ok(None) => return false,
        Err(e) => {
            warn!(error=%e, "插件命令匹配失败");
            return true;
        }
    };
    let Some(engine) = deps.reply.as_ref() else {
        return true;
    };
    let Ok((chat_type, target)) = reply_engine_route(m) else {
        return true;
    };
    let (skill, body) = command;
    let ctx = crate::tools::ToolCtx {
        task_id: None,
        chat_id: m.chat_id.clone(),
        chat_type: m.chat_type.clone(),
        sender_pid: m.sender_pid.clone(),
        locale: Some("zh-CN".into()),
    };
    let mut slots = serde_json::json!({"query":body,"user_text":body,"explicit_command":true});
    add_message_slots(deps, m, &mut slots).await;
    let result = if body.is_empty()
        && m.reply_to.is_none()
        && slots["mentioned_qqs"]
            .as_array()
            .is_none_or(|a| a.is_empty())
    {
        Err(anyhow::anyhow!(
            "请填写命令参数；撤回可引用目标消息后发送 /撤回"
        ))
    } else {
        skill.invoke(&ctx, slots).await
    };
    let (text, artifacts) = match result {
        Ok(out) => (
            out.data["rendered"]
                .as_str()
                .unwrap_or(&out.summary)
                .to_string(),
            out.artifacts,
        ),
        Err(e) => (format!("命令未完成：{e}"), vec![]),
    };
    engine.enqueue(reply_engine::ReplyJob {
        chat_id: m.chat_id.clone(),
        chat_type,
        target,
        anchor_msg_id: m.msg_id,
        mention: false,
        mention_qq: None,
        kind: reply_engine::JobKind::Bubbles(vec![reply_engine::Bubble {
            text,
            at: false,
            meme: None,
        }]),
    });
    for path in artifacts {
        if let Err(e) = engine.enqueue_attachment_with_retention(
            m.chat_id.clone(),
            chat_type,
            target,
            path.into(),
            skill.artifact_retention(),
        ) {
            warn!(error=%e, "插件命令附件入队失败");
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn plugin_command_works_without_models_and_keeps_output_as_text() {
        struct Search {
            artifact: String,
        }
        #[async_trait::async_trait]
        impl crate::skills::Skill for Search {
            fn def(&self) -> crate::skills::SkillDef {
                crate::skills::SkillDef {
                    name: "test.search",
                    description: "test",
                    prompt_template: "".into(),
                    tools: vec![],
                    role_hint: None,
                }
            }
            fn commands(&self) -> &'static [&'static str] {
                &["/搜索", "/下载", "/撤回"]
            }
            async fn invoke(
                &self,
                _: &crate::tools::ToolCtx,
                slots: serde_json::Value,
            ) -> anyhow::Result<crate::tools::ToolOutput> {
                assert_eq!(slots["explicit_command"], true);
                assert_eq!(slots["user_text"], slots["query"]);
                if slots["user_text"] == "" {
                    assert_eq!(slots["reply_to"], -321);
                    assert_eq!(slots["mentioned_qqs"], serde_json::json!([456]));
                    return Ok(crate::tools::ToolOutput {
                        summary: "已撤回".into(),
                        artifacts: vec![],
                        data: serde_json::json!({}),
                    });
                }
                if slots["query"] == "https://www.bilibili.com/video/BV1xx411c7mD" {
                    return Ok(crate::tools::ToolOutput {
                        summary: "下载完成，开始交付".into(),
                        artifacts: vec![self.artifact.clone()],
                        data: serde_json::json!({}),
                    });
                }
                assert_eq!(slots["query"], "Rust");
                Ok(crate::tools::ToolOutput {
                    summary: "Rust\n::meme 开心\nhttps://www.rust-lang.org/".into(),
                    artifacts: vec![],
                    data: serde_json::json!({}),
                })
            }
        }
        let db = temp_db().await;
        let artifact = db.parent().unwrap().join("command.mp4");
        std::fs::write(&artifact, b"video").unwrap();
        let sent = Arc::new(std::sync::Mutex::new(Vec::new()));
        let capture = sent.clone();
        let send: reply_engine::SendFn = Arc::new(move |request| {
            capture.lock().unwrap().push(request);
            Box::pin(async { Ok(serde_json::json!({"message_id":1})) })
        });
        let bus = EventBus::default();
        let mood = MoodState::default();
        let cfg = Arc::new(RwLock::new(reply_engine::ReplyCfg {
            first_delay_min_ms: 0,
            first_delay_max_ms: 0,
            min_delay_ms: 0,
            max_delay_ms: 0,
            ..Default::default()
        }));
        let engine = reply_engine::ReplyEngine::new(
            db.clone(),
            bus.clone(),
            send,
            SelfMsgIds::default(),
            cfg.clone(),
            mood.clone(),
        )
        .spawn();
        let skills = crate::skills::SkillRegistry::new();
        skills.register_arc(Arc::new(Search {
            artifact: artifact.to_string_lossy().into_owned(),
        }));
        let deps = PipelineDeps {
            bus,
            db_path: db,
            llm: Arc::new(RwLock::new(None)),
            self_qq: Arc::new(AtomicU64::new(100)),
            self_ids: SelfMsgIds::default(),
            mood,
            prefilter: Arc::new(RwLock::new(Default::default())),
            reply: Some(engine.clone()),
            reply_cfg: cfg,
            ctx_cfg: Arc::new(RwLock::new(Default::default())),
            memes_dir: PathBuf::new(),
            skill_registry: Some(skills),
            per_chat_cap: Arc::new(RwLock::new(32)),
        };
        let mut message = MessageReceivedPayload {
            msg_id: 0,
            chat_id: "42".into(),
            chat_type: "group".into(),
            sender_pid: "p_1".into(),
            text: "/搜索 Rust".into(),
            at_me: false,
            has_image: false,
            reply_to: None,
            sender_bot: false,
            image_urls: vec![],
            ts: 1,
        };
        handle_inner(&deps, &message, 1).await;
        message.text = "/下载 https://www.bilibili.com/video/BV1xx411c7mD".into();
        handle_inner(&deps, &message, 1).await;
        let mut conn = crate::db::connect(&deps.db_path).await.unwrap();
        conn.execute(
            "INSERT INTO persons(person_id,first_seen,last_seen) VALUES ('p_1',1,1)",
            crate::db::params![],
        )
        .await
        .unwrap();
        conn.execute("INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts,mentions) VALUES ('42','group','p_1','/撤回',1,?1)",crate::db::params!["[\"p_100\",\"p_456\",\"p_456\"]"]).await.unwrap();
        message.msg_id = conn.last_insert_rowid().await.unwrap();
        drop(conn);
        message.text = "/撤回".into();
        message.reply_to = Some(-321);
        handle_inner(&deps, &message, 1).await;
        message.sender_pid = "self".into();
        assert!(!handle_plugin_command(&deps, &message).await);
        message.sender_pid = "p_100".into();
        assert!(!handle_plugin_command(&deps, &message).await);
        assert!(engine.shutdown(std::time::Duration::from_secs(3)).await);
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 4);
        assert_eq!(sent[0].segments[0]["type"], "text");
        assert!(sent[0].segments[0]["data"]["text"]
            .as_str()
            .unwrap()
            .contains("::meme"));
        assert_eq!(sent[1].segments[0]["data"]["text"], "下载完成，开始交付");
        assert_eq!(sent[2].segments[0]["type"], "video");
        assert_eq!(sent[3].segments[0]["data"]["text"], "已撤回");
    }
    use crate::db::params;

    async fn temp_db() -> std::path::PathBuf {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nanos = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed) as u128
            + (std::process::id() as u128)
            << 16;
        let dir = std::env::temp_dir().join(format!("yt-bot-test-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("yuantuan.db");
        let mut conn = crate::db::connect(&db).await.unwrap();
        crate::db::migrate(&mut conn).await.unwrap();
        db
    }

    /// Q55：processed_at 回写——任意 msg_id 调用后该行不再 NULL
    #[tokio::test]
    async fn mark_processed_writes_ts() {
        let db = temp_db().await;
        let mut conn = crate::db::connect(&db).await.unwrap();
        let now = 1000i64;
        conn.execute(
            "INSERT INTO persons(person_id, display_name, first_seen, last_seen) VALUES ('p_1','甲',?1,?1)",
            params![now],
        ).await.unwrap();
        conn.execute(
            "INSERT INTO messages(chat_id, chat_type, sender_pid, text, ts) VALUES ('c1','group','p_1','hi',?1)",
            params![now],
        ).await.unwrap();
        let msg_id = conn.last_insert_rowid().await.unwrap();
        drop(conn);

        mark_processed(&db, msg_id).await;

        let mut conn = crate::db::connect(&db).await.unwrap();
        let processed: Option<i64> = conn
            .query_row(
                "SELECT processed_at FROM messages WHERE msg_id = ?1",
                params![msg_id],
                |r| r.try_get(0),
            )
            .await
            .unwrap();
        assert!(processed.is_some(), "processed_at 应被回写");
    }

    /// Q55：回放扫描——self 消息不进回放集；/image 命令跳过并标记
    #[tokio::test]
    async fn replay_skips_self_and_image_command() {
        let db = temp_db().await;
        // 用当前时间避免被 1h 回放窗口丢弃(REPLAY_WINDOW_SECS)
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
            - 60; // 一分钟前(窗口内)
        {
            let mut conn = crate::db::connect(&db).await.unwrap();
            conn.execute(
                "INSERT INTO persons(person_id, display_name, first_seen, last_seen) VALUES ('self','云团',?1,?1),('p_1','甲',?1,?1)",
                params![now],
            ).await.unwrap();
            // self 消息（应被回放扫描排除）
            conn.execute(
                "INSERT INTO messages(chat_id, chat_type, sender_pid, text, ts) VALUES ('c1','group','self','我自己说的',?1)",
                params![now],
            ).await.unwrap();
            // /image 直派命令（回放应跳过且直接标记已处理）
            conn.execute(
                "INSERT INTO messages(chat_id, chat_type, sender_pid, text, ts) VALUES ('c1','group','p_1','/image 一只猫',?1)",
                params![now + 1],
            ).await.unwrap();
        }

        // 构造最简 PipelineDeps：无 LLM、无 reply、无 skills
        let bus = crate::event::EventBus::default();
        let llm_slot: SharedLlm = std::sync::Arc::new(std::sync::RwLock::new(None));
        let prefilter_slot: SharedPrefilter =
            std::sync::Arc::new(std::sync::RwLock::new(crate::prefilter::Config::default()));
        let reply_slot: crate::reply_engine::SharedReplyCfg = std::sync::Arc::new(
            std::sync::RwLock::new(crate::reply_engine::ReplyCfg::default()),
        );
        let ctx_slot: crate::context_builder::SharedContextCfg = std::sync::Arc::new(
            std::sync::RwLock::new(crate::context_builder::ContextCfg::default()),
        );
        let deps = PipelineDeps {
            bus: bus.clone(),
            db_path: db.clone(),
            llm: llm_slot,
            self_qq: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            self_ids: crate::prefilter::SelfMsgIds::default(),
            mood: crate::state::MoodState::default(),
            prefilter: prefilter_slot,
            reply: None,
            reply_cfg: reply_slot,
            ctx_cfg: ctx_slot,
            memes_dir: std::path::PathBuf::from("data/memes"),
            skill_registry: None,
            per_chat_cap: std::sync::Arc::new(std::sync::RwLock::new(32)),
        };

        let n = replay_pending(&deps).await.unwrap();
        assert_eq!(n, 0, "self 与 /image 都不应回放（回放计数为 0）");

        // /image 那条也应被标记为已处理
        let mut conn = crate::db::connect(&db).await.unwrap();
        let pending: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE processed_at IS NULL AND sender_pid != 'self'",
                crate::db::params![],
                |r| r.try_get(0),
            )
            .await
            .unwrap();
        assert_eq!(pending, 0, "回放后不应剩余未处理非 self 消息");
    }

    /// Q52 信号量上限常量为 4（runtime-design 裁决值）
    #[test]
    fn q53_concurrency_limit_is_four() {
        assert_eq!(crate::llm::DEFAULT_LLM_CONCURRENCY, 4);
    }

    /// Q55+1h 窗口:远古未处理消息被放弃(标记为已处理),不回放
    #[tokio::test]
    async fn replay_abandons_stale_pending() {
        let db = temp_db().await;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        {
            let mut conn = crate::db::connect(&db).await.unwrap();
            conn.execute(
                "INSERT INTO persons(person_id, display_name, first_seen, last_seen) VALUES ('p_1','甲',?1,?1)",
                params![now],
            ).await.unwrap();
            // 远古:2 小时前的未处理消息
            let stale = now - 7200;
            conn.execute(
                "INSERT INTO messages(chat_id, chat_type, sender_pid, text, ts) VALUES ('c1','group','p_1','远古消息',?1)",
                params![stale],
            ).await.unwrap();
        }
        let deps = PipelineDeps {
            bus: crate::event::EventBus::default(),
            db_path: db.clone(),
            llm: std::sync::Arc::new(std::sync::RwLock::new(None)),
            self_qq: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            self_ids: crate::prefilter::SelfMsgIds::default(),
            mood: crate::state::MoodState::default(),
            prefilter: std::sync::Arc::new(std::sync::RwLock::new(
                crate::prefilter::Config::default(),
            )),
            reply: None,
            reply_cfg: std::sync::Arc::new(std::sync::RwLock::new(
                crate::reply_engine::ReplyCfg::default(),
            )),
            ctx_cfg: std::sync::Arc::new(std::sync::RwLock::new(
                crate::context_builder::ContextCfg::default(),
            )),
            memes_dir: std::path::PathBuf::from("data/memes"),
            skill_registry: None,
            per_chat_cap: std::sync::Arc::new(std::sync::RwLock::new(32)),
        };
        let n = replay_pending(&deps).await.unwrap();
        assert_eq!(n, 0, "远古消息应被放弃,不回放");
        let mut conn = crate::db::connect(&db).await.unwrap();
        let pending: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE processed_at IS NULL",
                crate::db::params![],
                |r| r.try_get(0),
            )
            .await
            .unwrap();
        assert_eq!(pending, 0, "远古未处理应被 UPDATE 标记已处理");
    }
}
