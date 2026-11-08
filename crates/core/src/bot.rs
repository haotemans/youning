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

/// 即时队列按 (chat_type, chat_id) 串行，容量在 worker 建立时读取。
/// 普通群聊批次独立运行；队列满则保留 SQLite 待处理状态，由周期补读。
pub type SharedPerChatCap = Arc<RwLock<usize>>;

/// Startup only repairs durable states; it never replays a model or paid command
/// per historical message. The periodic reader resumes unread input in batches.
pub async fn replay_pending(deps: &PipelineDeps) -> anyhow::Result<usize> {
    crate::window::recover(&deps.db_path).await
}

type Conversation = (String, String);
type DirectLanes =
    std::collections::HashMap<Conversation, tokio::sync::mpsc::Sender<MessageReceivedPayload>>;
type PipelineWorkers = tokio::task::JoinSet<Option<Conversation>>;

fn is_immediate(deps: &PipelineDeps, m: &MessageReceivedPayload) -> bool {
    is_direct_message(m, &deps.self_ids)
        || is_image_command(&m.text)
        || matches!(match_plugin_message(deps, m), Ok(Some(_)))
}

async fn dispatch_direct(
    deps: &Arc<PipelineDeps>,
    lanes: &mut DirectLanes,
    workers: &mut PipelineWorkers,
    m: MessageReceivedPayload,
) {
    if !crate::window::claim(&deps.db_path, m.msg_id)
        .await
        .unwrap_or(false)
    {
        return;
    }
    let id = m.msg_id;
    let key = (m.chat_type.clone(), m.chat_id.clone());
    let tx = lanes.entry(key).or_insert_with(|| {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<MessageReceivedPayload>(
            (*deps.per_chat_cap.read().unwrap()).max(1),
        );
        let deps = Arc::clone(deps);
        workers.spawn(async move {
            while let Some(m) = rx.recv().await {
                handle_claimed(&deps, &m, m.msg_id).await;
            }
            None
        });
        tx
    });
    if tx.try_send(m).is_err() {
        // SQLite remains pending; the next tick retries without blocking @ in
        // other conversations or spawning an unbounded overflow worker.
        let _ = crate::window::release(&deps.db_path, &[id]).await;
        warn!("即时队列已满，消息保留在 SQLite 等待补读");
    }
}

pub fn spawn_pipeline(deps: PipelineDeps) -> JoinHandle<()> {
    spawn_pipeline_with_period(
        deps,
        std::time::Duration::from_secs(crate::window::DEFAULT_POLL_SECS),
    )
}

fn spawn_pipeline_with_period(deps: PipelineDeps, period: std::time::Duration) -> JoinHandle<()> {
    let mut rx = deps.bus.subscribe();
    tokio::spawn(async move {
        if let Err(e) = crate::window::recover(&deps.db_path).await {
            warn!(%e,"恢复消息状态失败");
            return;
        }
        let startup_cutoff = match crate::db::connect(&deps.db_path).await {
            Ok(mut conn) => conn
                .query_row(
                    "SELECT COALESCE(MAX(msg_id),0) FROM messages",
                    crate::db::params![],
                    |r| r.try_get::<i64, _>(0),
                )
                .await
                .unwrap_or(0),
            Err(_) => 0,
        };
        let deps = Arc::new(deps);
        let mut lanes = DirectLanes::new();
        let mut busy = std::collections::HashSet::new();
        let periodic_slots = Arc::new(tokio::sync::Semaphore::new(
            crate::window::DEFAULT_PERIODIC_CONCURRENCY,
        ));
        let mut workers = PipelineWorkers::new();
        let delivery_deps = Arc::clone(&deps);
        workers.spawn(async move {
            crate::agent::delivery::run(delivery_deps).await;
            None
        });
        let mut clock = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        info!("Decision 管线启动：群聊每30秒增量判断，定向消息即时处理");
        loop {
            tokio::select! {
                event = rx.recv() => match event {
                    Ok(Event::MessageReceived(m)) => {
                        let self_qq = deps.self_qq.load(Ordering::Relaxed);
                        if m.sender_bot || m.sender_pid == "self" || (self_qq != 0 && m.sender_pid == format!("p_{self_qq}")) {
                            mark_processed(&deps.db_path, m.msg_id).await;
                        } else if is_immediate(&deps, &m) {
                            dispatch_direct(&deps, &mut lanes, &mut workers, m).await;
                        }
                        // Ordinary messages were persisted by ingest. No timer
                        // reset, model request, or processed flag on arrival.
                    }
                    Ok(_) => {},
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => warn!(skipped=n,"消息事件滞后，将由 SQLite 增量补读"),
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
                _ = clock.tick() => {
                    let batches = match crate::window::pending(&deps.db_path).await {
                        Ok(batches) => batches,
                        Err(e) => { warn!(%e,"无法读取待判断消息"); continue; }
                    };
                    for mut batch in batches {
                        let mut ordinary = vec![];
                        for m in batch.messages {
                            if is_immediate(&deps, &m) {
                                // On restart never automatically replay commands
                                // with external effects whose prior receipt is unknown.
                                if m.msg_id <= startup_cutoff && (is_image_command(&m.text) || matches!(match_plugin_message(&deps,&m),Ok(Some(_)))) {
                                    mark_uncertain(&deps.db_path,m.msg_id).await;
                                } else { dispatch_direct(&deps,&mut lanes,&mut workers,m).await; }
                            } else { ordinary.push(m); }
                        }
                        if ordinary.is_empty() { continue; }
                        let key = (ordinary[0].chat_type.clone(),ordinary[0].chat_id.clone());
                        if !busy.insert(key.clone()) { continue; }
                        batch.context.retain(|c| ordinary.iter().any(|m| m.msg_id==c.msg_id));
                        batch.messages = ordinary;
                        let deps = Arc::clone(&deps);
                        let slots=Arc::clone(&periodic_slots);
                        workers.spawn(async move {
                            if let Ok(_permit)=slots.acquire_owned().await { handle_periodic(&deps,batch).await; }
                            Some(key)
                        });
                    }
                },
                result = workers.join_next(), if !workers.is_empty() => {
                    match result {
                        Some(Ok(Some(key))) => { busy.remove(&key); },
                        Some(Err(e)) => {
                            // A panicking worker can have an unknown side effect;
                            // stop this pipeline so startup recovery can distinguish it.
                            warn!(%e,"消息 worker 异常退出，交给管线恢复"); break;
                        }
                        _ => {},
                    }
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

fn is_direct_message(m: &MessageReceivedPayload, self_ids: &SelfMsgIds) -> bool {
    m.at_me
        || m.chat_type == "private"
        || m.reply_to.is_some_and(|id| self_ids.contains(id))
        || m.text.trim_start().starts_with("云团")
        || crate::decision::is_reply_attention_request(&m.text)
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

async fn handle_claimed(deps: &PipelineDeps, m: &MessageReceivedPayload, cutoff: i64) {
    if let Some(failure) = handle_inner(deps, m, cutoff).await {
        if let Err(e) = crate::window::fail(
            &deps.db_path,
            &[m.msg_id],
            &failure.reason,
            failure.terminal,
        )
        .await
        {
            warn!(%e,"无法保存判断失败状态");
        }
    } else {
        mark_processed(&deps.db_path, m.msg_id).await;
    }
}
fn pending_failure(reason: &str) -> Option<decision::DecisionFailure> {
    Some(decision::DecisionFailure {
        reason: reason.into(),
        terminal: false,
    })
}

async fn mark_processed(db_path: &std::path::Path, msg_id: i64) {
    if msg_id > 0 {
        if let Err(e) = crate::window::complete(db_path, &[msg_id]).await {
            warn!(%e,msg_id,"无法保存消息处理状态");
        }
    }
}

async fn mark_uncertain(path: &std::path::Path, id: i64) {
    if let Ok(mut conn) = crate::db::connect(path).await {
        let _=conn.execute("UPDATE messages SET decision_state='uncertain',processed_at=?1 WHERE msg_id=?2 AND decision_state='pending' AND processed_at IS NULL",crate::db::params![crate::window::now(),id]).await;
    }
}

async fn handle_inner(
    deps: &PipelineDeps,
    m: &MessageReceivedPayload,
    cutoff: i64,
) -> Option<decision::DecisionFailure> {
    if is_image_command(&m.text) || matches!(match_plugin_message(deps, m), Ok(Some(_))) {
        if crate::window::executing(&deps.db_path, &[m.msg_id])
            .await
            .is_err()
        {
            return pending_failure("回复上下文或处理状态暂时不可用");
        }
    }
    if handle_plugin_command(deps, m).await {
        return None;
    }
    // 生图只由已启用的 NAI 插件处理；不再回退到旧 Core 媒体实现。
    if is_image_command(&m.text) {
        enqueue_image_plugin_disabled(deps, m);
        return None;
    }

    let q = deps.self_qq.load(Ordering::Relaxed);
    let self_pid = (q != 0).then(|| format!("p_{q}"));
    let pf = *deps.prefilter.read().unwrap(); // 热应用：每条取当前阈值
    match crate::prefilter::check(&pf, self_pid.as_deref(), &deps.self_ids, &deps.db_path, m).await
    {
        Verdict::Drop(rule) => debug!(rule, chat_id = %m.chat_id, "Prefilter 丢弃"),
        Verdict::Pass => {
            // A bare mention is a greeting, not an expensive model request. Keep
            // self/bot filtering above and preserve image/quoted-message context.
            if is_bare_mention(m) {
                enqueue_direct_reply(deps, m, "在呢，想让我帮你做什么？");
                return None;
            }
            let gw = match deps.llm.read().unwrap().clone() {
                Some(g) if g.role(Role::Decision).is_some() => g,
                _ => {
                    debug!(chat_id = %m.chat_id, "Decision 角色不可用");
                    return Some(decision::DecisionFailure {
                        reason: "Decision角色未配置，请在模型服务绑定模型".into(),
                        terminal: true,
                    });
                }
            };
            // 成本闸热应用：prefilter 槽 → gateway（每条对齐一次，换槽/调参即生效）
            gw.set_cost_per_min(pf.decision_cost_per_min.max(1) as usize);
            let ctx_cfg = *deps.ctx_cfg.read().unwrap();
            let mut snapshot = match crate::context_builder::capture_reply_snapshot(
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
                    return pending_failure("回复上下文或处理状态暂时不可用");
                }
            };
            if decision::is_directed(m, &snapshot) {
                match tokio::time::timeout(
                    // Up to three bounded 40s image observations, plus DB work.
                    std::time::Duration::from_secs(125),
                    crate::vision::observe(&deps.db_path, &gw, m, &snapshot),
                )
                .await
                {
                    Ok(Ok(false)) => {}
                    result => {
                        if !matches!(result, Ok(Ok(true))) {
                            warn!(chat_id=%m.chat_id,"识图未完成，使用明确标记的缺失/失败语境");
                        }
                        if let Ok(updated) = crate::context_builder::capture_reply_snapshot(
                            &deps.db_path,
                            m,
                            cutoff,
                            &ctx_cfg,
                        )
                        .await
                        {
                            snapshot = updated;
                        }
                    }
                }
            }
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

            if outcome.fallback {
                return outcome
                    .failure
                    .or_else(|| pending_failure("Decision判断失败"));
            }
            if crate::window::executing(&deps.db_path, &[m.msg_id])
                .await
                .is_err()
            {
                return pending_failure("回复上下文或处理状态暂时不可用");
            }
            execute_outcome(deps, m, snapshot, gw, outcome, ctx_cfg).await;
        }
    }
    None
}

async fn execute_outcome(
    deps: &PipelineDeps,
    m: &MessageReceivedPayload,
    mut snapshot: crate::context_builder::ReplySnapshot,
    gw: Arc<LlmGateway>,
    mut outcome: decision::DecisionOutcome,
    ctx_cfg: crate::context_builder::ContextCfg,
) {
    for route_turn in 0..3 {
        if let Some(query) = outcome.output.history_query.as_deref() {
            if let Err(e) =
                crate::context_builder::retrieve_history(&deps.db_path, &mut snapshot, query).await
            {
                warn!(%e,"历史检索失败");
            }
        }
        if let Some(control) = outcome.output.task_control.as_ref() {
            if decision::is_directed(m, &snapshot)
                && !control.request_quote.trim().is_empty()
                && m.text.contains(control.request_quote.trim())
            {
                let result = crate::agent::runtime::control(
                    &deps.db_path,
                    &control.task_id,
                    &m.sender_pid,
                    &m.chat_id,
                    &m.chat_type,
                    &control.action,
                    &m.text,
                )
                .await;
                snapshot.execution_results.push(serde_json::json!({"task_control":control.action,"ok":result.is_ok(),"error":result.err().map(|e|e.to_string())}));
            }
            outcome.output.action = DecisionAction::Reply;
        }
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
            match crate::capabilities::invoke(&deps.db_path, &skill, &ctx, Some(m.msg_id), slots)
                .await
            {
                Ok(out) => {
                    if skill.is_information_lookup() && out.artifacts.is_empty() {
                        snapshot.execution_results.push(serde_json::json!({"skill":skill_name,"summary":out.summary,"data":out.data}));
                        outcome.output.action = DecisionAction::Reply;
                        outcome.output.history_query = None;
                        continue;
                    }
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
                    let mention_qq = plugin_result_mention(m, &out);
                    if let Err(e) = engine.enqueue_plugin_result_with_mention(
                        m.chat_id.clone(),
                        chat_type,
                        target,
                        m.msg_id,
                        rendered,
                        out.artifacts,
                        skill.artifact_retention(),
                        skill.delivery_policy(),
                        mention_qq,
                    ) {
                        warn!(error=%e, "插件结果入队失败");
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
                        let picked = meme::pick(&deps.db_path, &deps.memes_dir, &cat).await;
                        match picked {
                            Some(path) => {
                                info!(chat_id = %m.chat_id, category = %cat, "send_meme 抽图入队");
                                engine.enqueue_image(m.chat_id.clone(), chat_type, target, path);
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
            if let Err(e) = crate::agent::create_scoped_task(
                    &deps.db_path,&deps.bus,&m.chat_id,&m.chat_type,&m.sender_pid,Some(m.msg_id),&goal,
                    outcome.output.task_plan.clone().unwrap_or_default(),
                    serde_json::json!({"anchor":snapshot.anchor,"quoted_messages":snapshot.quoted_messages,"recent_messages":snapshot.recent_messages,"person_profile":snapshot.profile,"group_profile":snapshot.group_profile,"history":snapshot.historical_messages}),
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
                let catalog = if let Some(registry) = deps.skill_registry.as_ref() {
                    let ctx = crate::tools::ToolCtx {
                        task_id: None,
                        chat_id: m.chat_id.clone(),
                        chat_type: m.chat_type.clone(),
                        sender_pid: m.sender_pid.clone(),
                        locale: None,
                    };
                    crate::capabilities::visible_skills(&deps.db_path, registry, &ctx)
                        .await
                        .unwrap_or_default()
                } else {
                    vec![]
                };
                if let Err(e) = reply_engine::prepare_and_enqueue(
                    engine,
                    &snapshot,
                    &gw,
                    &deps.mood,
                    m,
                    &outcome.output,
                    &ctx_cfg,
                    &reply_cfg,
                    &catalog,
                )
                .await
                {
                    if let Some(request) = e.downcast_ref::<reply_engine::ExecutionRequest>() {
                        if route_turn < 2 && snapshot.execution_request.is_none() {
                            snapshot.execution_request =
                                Some(serde_json::json!({"goal":request.0,"anchor_id":m.msg_id}));
                            outcome = decision::decide(
                                &deps.db_path,
                                &gw,
                                &deps.bus,
                                &deps.mood,
                                m,
                                deps.skill_registry.as_ref(),
                                &snapshot,
                            )
                            .await;
                            continue;
                        }
                        enqueue_direct_reply(
                            deps,
                            m,
                            "这一步还缺少明确的执行条件，请补充具体要完成的内容。",
                        );
                        return;
                    }
                    warn!(chat_id = %m.chat_id, error = %format!("{e:#}"), "bot_chat 回复准备失败");
                    deps.bus.publish(Event::BubbleSent(BubbleSentPayload {
                        chat_id: m.chat_id.clone(),
                        bubble_index: 0,
                        total: 0,
                        ok: false,
                        note: Some(format!("bot_chat 调用失败: {e:#}")),
                    }));
                    if m.at_me || m.chat_type == "private" {
                        enqueue_direct_reply(deps, m, "刚才没能生成回复，请稍后再试一次。");
                    }
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
        break;
    }
}

async fn handle_periodic(deps: &PipelineDeps, mut batch: crate::window::PendingBatch) {
    if !decision_ready(&deps.llm) {
        return;
    }
    let mut claimed = vec![];
    let q = deps.self_qq.load(Ordering::Relaxed);
    let self_pid = (q != 0).then(|| format!("p_{q}"));
    let pf = *deps.prefilter.read().unwrap();
    for m in batch.messages {
        if !crate::window::claim(&deps.db_path, m.msg_id)
            .await
            .unwrap_or(false)
        {
            continue;
        }
        if matches!(
            crate::prefilter::check(&pf, self_pid.as_deref(), &deps.self_ids, &deps.db_path, &m)
                .await,
            Verdict::Pass
        ) {
            claimed.push(m);
        } else {
            mark_processed(&deps.db_path, m.msg_id).await;
        }
    }
    if claimed.is_empty() {
        return;
    }
    batch
        .context
        .retain(|c| claimed.iter().any(|m| m.msg_id == c.msg_id));
    batch.messages = claimed;
    let ids = batch.messages.iter().map(|m| m.msg_id).collect::<Vec<_>>();
    let cfg = *deps.ctx_cfg.read().unwrap();
    let gw = deps.llm.read().unwrap().clone();
    let result = async {
        let gw = gw.ok_or_else(|| anyhow::anyhow!("Decision 角色不可用"))?;
        gw.set_cost_per_min(pf.decision_cost_per_min.max(1) as usize);
        let (m, snapshot, outcome) = decision::decide_periodic(
            &deps.db_path,
            &gw,
            &deps.bus,
            &deps.mood,
            &batch,
            deps.skill_registry.as_ref(),
            &cfg,
        )
        .await?;
        if outcome.fallback {
            return Err(anyhow::Error::new(outcome.failure.unwrap_or(
                decision::DecisionFailure {
                    reason: "周期判断失败".into(),
                    terminal: false,
                },
            )));
        }
        if outcome.output.action != DecisionAction::Ignore
            && crate::window::interrupted(&deps.db_path, &m.chat_type, &m.chat_id, batch.cutoff)
                .await?
        {
            crate::window::complete(&deps.db_path, &ids).await?;
            info!("本批主动回应已让位给新对话，不再次重排旧消息");
            return Ok(());
        }
        // Claim all observed messages before any external action. A crash during
        // execution requires verification; it must not repeat an old paid action.
        crate::window::executing(&deps.db_path, &ids).await?;
        execute_outcome(deps, &m, snapshot, gw, outcome, cfg).await;
        crate::window::complete(&deps.db_path, &ids).await
    }
    .await;
    if let Err(e) = result {
        warn!(%e,"周期判断未完成，保留待处理范围");
        let failure = result_failure(&e);
        if let Err(e) =
            crate::window::fail(&deps.db_path, &ids, &failure.reason, failure.terminal).await
        {
            warn!(%e,"无法保存批次重试状态");
        }
    }
}

fn result_failure(error: &anyhow::Error) -> decision::DecisionFailure {
    error
        .downcast_ref::<decision::DecisionFailure>()
        .cloned()
        .unwrap_or(decision::DecisionFailure {
            reason: "判断上下文或数据库读取失败".into(),
            terminal: false,
        })
}

fn is_bare_mention(m: &MessageReceivedPayload) -> bool {
    m.at_me && m.text.trim().is_empty() && !m.has_image && m.reply_to.is_none()
}

fn enqueue_direct_reply(deps: &PipelineDeps, m: &MessageReceivedPayload, text: &str) {
    let Some(engine) = deps.reply.as_ref() else {
        return;
    };
    let Ok((chat_type, target, mention_qq)) = reply_engine::route_of(m) else {
        return;
    };
    engine.enqueue(reply_engine::ReplyJob {
        chat_id: m.chat_id.clone(),
        chat_type,
        target,
        anchor_msg_id: m.msg_id,
        mention: m.chat_type == "group",
        mention_qq,
        kind: reply_engine::JobKind::Bubbles(vec![reply_engine::Bubble {
            text: text.to_owned(),
            at: false,
            meme: None,
        }]),
    });
}

/// Explicit plugin commands use registered, permission-checked skills even without LLM roles.
async fn add_message_slots(
    deps: &PipelineDeps,
    m: &MessageReceivedPayload,
    slots: &mut serde_json::Value,
) {
    slots["original_user_text"] = serde_json::json!(m.text);
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

fn match_plugin_message(
    deps: &PipelineDeps,
    m: &MessageReceivedPayload,
) -> anyhow::Result<Option<(Arc<dyn crate::skills::Skill>, serde_json::Value)>> {
    let own_qq = deps.self_qq.load(Ordering::Relaxed);
    if m.sender_bot
        || m.sender_pid == "self"
        || (own_qq != 0 && m.sender_pid == format!("p_{own_qq}"))
    {
        return Ok(None);
    }
    let Some(registry) = deps.skill_registry.as_ref() else {
        return Ok(None);
    };
    if let Some((skill, body)) = registry.command(&m.text)? {
        return Ok(Some((
            skill,
            serde_json::json!({"query":body,"user_text":body,"explicit_command":true}),
        )));
    }
    let ctx = crate::tools::ToolCtx {
        task_id: None,
        chat_id: m.chat_id.clone(),
        chat_type: m.chat_type.clone(),
        sender_pid: m.sender_pid.clone(),
        locale: Some("zh-CN".into()),
    };
    registry.match_message(&ctx, &m.text)
}

/// Plugins can request an @, but its target always comes from the actual sender.
fn plugin_result_mention(
    message: &MessageReceivedPayload,
    output: &crate::tools::ToolOutput,
) -> Option<u64> {
    if message.chat_type != "group"
        || output.artifacts.is_empty()
        || output.data["mention_sender"] != true
    {
        return None;
    }
    message
        .sender_pid
        .strip_prefix("p_")?
        .parse()
        .ok()
        .filter(|qq| *qq > 0)
}

async fn handle_plugin_command(deps: &PipelineDeps, m: &MessageReceivedPayload) -> bool {
    let (skill, mut slots) = match match_plugin_message(deps, m) {
        Ok(Some(matched)) => matched,
        Ok(None) => return false,
        Err(e) => {
            warn!(error=%e, "插件消息匹配失败");
            return true;
        }
    };
    let Some(engine) = deps.reply.as_ref() else {
        return true;
    };
    let Ok((chat_type, target)) = reply_engine_route(m) else {
        return true;
    };
    let ctx = crate::tools::ToolCtx {
        task_id: None,
        chat_id: m.chat_id.clone(),
        chat_type: m.chat_type.clone(),
        sender_pid: m.sender_pid.clone(),
        locale: Some("zh-CN".into()),
    };
    add_message_slots(deps, m, &mut slots).await;
    let automatic = slots["automatic"].as_bool().unwrap_or(false);
    let result = if slots["explicit_command"] == true
        && slots["user_text"].as_str().is_some_and(str::is_empty)
        && m.reply_to.is_none()
        && slots["mentioned_qqs"]
            .as_array()
            .is_none_or(|a| a.is_empty())
    {
        Err(anyhow::anyhow!(
            "请填写命令参数；撤回可引用目标消息后发送 /撤回"
        ))
    } else {
        crate::capabilities::invoke(&deps.db_path, &skill, &ctx, Some(m.msg_id), slots).await
    };
    let mut policy = yuantuan_plugin_api::DeliveryPolicy::default();
    let mut mention_qq = None;
    let (text, artifacts) = match result {
        Ok(out) => {
            policy = skill.delivery_policy();
            mention_qq = plugin_result_mention(m, &out);
            (
                out.data["rendered"]
                    .as_str()
                    .unwrap_or(&out.summary)
                    .to_string(),
                out.artifacts,
            )
        }
        Err(e) => (
            format!("{}未完成：{e}", if automatic { "解析下载" } else { "命令" }),
            vec![],
        ),
    };
    if let Err(e) = engine.enqueue_plugin_result_with_mention(
        m.chat_id.clone(),
        chat_type,
        target,
        m.msg_id,
        text,
        artifacts,
        skill.artifact_retention(),
        policy,
        mention_qq,
    ) {
        warn!(error=%e, "插件结果入队失败");
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn addressed_requests_do_not_enter_the_ordinary_group_window() {
        let mut message = MessageReceivedPayload {
            msg_id: 1,
            chat_id: "42".into(),
            chat_type: "group".into(),
            sender_pid: "p_456".into(),
            text: "禁言猫猫".into(),
            at_me: true,
            has_image: false,
            reply_to: None,
            sender_bot: false,
            image_urls: vec![],
            ts: 1,
        };
        let ids = SelfMsgIds::default();
        assert!(is_direct_message(&message, &ids));
        message.at_me = false;
        message.text = "为啥老是不理我".into();
        assert!(is_direct_message(&message, &ids));
        message.text.clear();
        message.has_image = true;
        assert!(!is_direct_message(&message, &ids));
        message.reply_to = Some(-12);
        assert!(!is_direct_message(&message, &ids));
        ids.record(-12);
        assert!(is_direct_message(&message, &ids));
        message.reply_to = None;
        message.chat_type = "private".into();
        assert!(is_direct_message(&message, &ids));
    }
    #[test]
    fn plugin_image_mention_uses_incoming_sender_and_requires_explicit_opt_in() {
        let mut message = MessageReceivedPayload {
            msg_id: 0,
            chat_id: "42".into(),
            chat_type: "group".into(),
            sender_pid: "p_456".into(),
            text: "/image adult reading".into(),
            at_me: false,
            has_image: false,
            reply_to: None,
            sender_bot: false,
            image_urls: vec![],
            ts: 1,
        };
        let mut output = crate::tools::ToolOutput {
            summary: "internal metadata".into(),
            artifacts: vec!["generated.png".into()],
            data: serde_json::json!({"rendered":"", "mention_sender":true, "sender_qq":999}),
        };
        assert_eq!(plugin_result_mention(&message, &output), Some(456));
        message.chat_type = "private".into();
        assert_eq!(plugin_result_mention(&message, &output), None);
        message.chat_type = "group".into();
        message.sender_pid = "self".into();
        assert_eq!(plugin_result_mention(&message, &output), None);
        message.sender_pid = "p_0".into();
        assert_eq!(plugin_result_mention(&message, &output), None);
        message.sender_pid = "p_456".into();
        output.data["mention_sender"] = serde_json::json!(false);
        assert_eq!(plugin_result_mention(&message, &output), None);
        output.data["mention_sender"] = serde_json::json!(true);
        output.artifacts.clear();
        assert_eq!(plugin_result_mention(&message, &output), None);
    }

    #[tokio::test]
    async fn plugin_command_works_without_models_and_keeps_output_as_text() {
        struct Search {
            artifact: String,
            automatic: Arc<tokio::sync::Notify>,
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
            fn match_message(
                &self,
                _: &crate::tools::ToolCtx,
                text: &str,
            ) -> Option<serde_json::Value> {
                text.contains("https://xhslink.cn/o/123")
                    .then(|| serde_json::json!({"user_text":text,"automatic":true}))
            }
            async fn invoke(
                &self,
                _: &crate::tools::ToolCtx,
                slots: serde_json::Value,
            ) -> anyhow::Result<crate::tools::ToolOutput> {
                if slots["automatic"] == true {
                    self.automatic.notify_one();
                    return Ok(crate::tools::ToolOutput {
                        summary: "自动解析".into(),
                        artifacts: vec![],
                        data: serde_json::json!({}),
                    });
                }
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
                    assert_eq!(
                        slots["original_user_text"],
                        "/下载 https://www.bilibili.com/video/BV1xx411c7mD"
                    );
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
        let automatic = Arc::new(tokio::sync::Notify::new());
        skills.register_arc(Arc::new(Search {
            artifact: artifact.to_string_lossy().into_owned(),
            automatic: automatic.clone(),
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
        let _ = handle_inner(&deps, &message, 1).await;
        message.text = "/下载 https://www.bilibili.com/video/BV1xx411c7mD".into();
        let _ = handle_inner(&deps, &message, 1).await;
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
        let _ = handle_inner(&deps, &message, 1).await;
        message.sender_pid = "self".into();
        assert!(!handle_plugin_command(&deps, &message).await);
        message.sender_pid = "p_100".into();
        assert!(!handle_plugin_command(&deps, &message).await);
        message.sender_pid = "p_1".into();
        message.text = "分享文案 https://xhslink.cn/o/123 查看原文".into();
        message.reply_to = None;
        let mut conn = crate::db::connect(&deps.db_path).await.unwrap();
        conn.execute("INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts) VALUES('42','group','p_1',?1,2)",crate::db::params![message.text]).await.unwrap();
        message.msg_id = conn.last_insert_rowid().await.unwrap();
        drop(conn);
        let pipeline = spawn_pipeline(deps.clone());
        deps.bus.publish(Event::MessageReceived(message));
        // Automatic links bypass the ten-second aggregation window and require no model.
        tokio::time::timeout(std::time::Duration::from_secs(2), automatic.notified())
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if sent.lock().unwrap().iter().any(|request| {
                    request.segments.as_array().is_some_and(|segments| {
                        segments
                            .iter()
                            .any(|segment| segment["data"]["text"] == "自动解析")
                    })
                }) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("actual automatic delivery before shutting the pipeline down");
        pipeline.abort();
        let _ = pipeline.await;
        let mut mention = MessageReceivedPayload {
            msg_id: 0,
            chat_id: "42".into(),
            chat_type: "group".into(),
            sender_pid: "p_1".into(),
            text: " \n".into(),
            at_me: true,
            has_image: false,
            reply_to: None,
            sender_bot: false,
            image_urls: vec![],
            ts: 1,
        };
        assert!(is_bare_mention(&mention));
        let _ = handle_inner(&deps, &mention, 1).await; // No LLM configured; still sends a greeting.
        mention.sender_pid = "self".into();
        let _ = handle_inner(&deps, &mention, 1).await; // Self and other bots must never receive it.
        mention.sender_pid = "p_2".into();
        mention.sender_bot = true;
        let _ = handle_inner(&deps, &mention, 1).await;
        mention.has_image = true;
        assert!(!is_bare_mention(&mention));
        mention.has_image = false;
        mention.reply_to = Some(123);
        assert!(!is_bare_mention(&mention));
        assert!(engine.shutdown(std::time::Duration::from_secs(3)).await);
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 6);
        assert_eq!(sent[0].segments[0]["type"], "text");
        assert!(sent[0].segments[0]["data"]["text"]
            .as_str()
            .unwrap()
            .contains("::meme"));
        assert_eq!(sent[1].segments[0]["data"]["text"], "下载完成，开始交付");
        assert_eq!(sent[2].segments[0]["type"], "video");
        assert_eq!(sent[3].segments[0]["data"]["text"], "已撤回");
        assert_eq!(sent[4].segments[0]["data"]["text"], "自动解析");
        assert_eq!(sent[5].segments[0]["type"], "at");
        assert_eq!(sent[5].segments[0]["data"]["qq"], "1");
        assert_eq!(
            sent[5].segments[1]["data"]["text"].as_str().unwrap().trim(),
            "在呢，想让我帮你做什么？"
        );
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

    #[tokio::test]
    async fn replay_stops_stale_auto_responses_but_keeps_history() {
        let db = temp_db().await;
        let mut conn = crate::db::connect(&db).await.unwrap();
        conn.execute(
            "INSERT INTO persons(person_id,first_seen,last_seen) VALUES ('p_1',1,1)",
            params![],
        )
        .await
        .unwrap();
        conn.execute("INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts) VALUES ('c1','group','p_1','昨天尚未读取的话',1)",params![]).await.unwrap();
        drop(conn);
        assert_eq!(crate::window::recover(&db).await.unwrap(), 0);
        let batches = crate::window::pending(&db).await.unwrap();
        assert!(batches.is_empty());
        let mut c = crate::db::connect(&db).await.unwrap();
        let (text, state): (String, String) = c
            .query_row("SELECT text,decision_state FROM messages", params![], |r| {
                Ok((r.try_get(0)?, r.try_get(1)?))
            })
            .await
            .unwrap();
        assert_eq!(text, "昨天尚未读取的话");
        assert_eq!(state, "failed");
    }

    /// Q52 信号量上限常量为 4（runtime-design 裁决值）
    #[test]
    fn q53_concurrency_limit_is_four() {
        assert_eq!(crate::llm::DEFAULT_LLM_CONCURRENCY, 4);
    }
}

#[cfg(test)]
#[path = "bot/dispatch_tests.rs"]
mod dispatch_tests;
