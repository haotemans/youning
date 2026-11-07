//! Durable completion -> Decision -> Chat -> outbox. Never replays the user's command.
use crate::{
    bot::PipelineDeps,
    db::{self, params, SqliteExt},
    event::MessageReceivedPayload,
    llm::Role,
};
use anyhow::Result;
use serde_json::json;
use sqlx::Row;
use std::sync::Arc;

pub async fn run(deps: Arc<PipelineDeps>) {
    // Preparing had no external effect. Recover it once; ambiguous sends stay pending verification.
    if let Ok(mut c) = db::connect(&deps.db_path).await {
        let _=c.execute("UPDATE task_runtime SET delivery_state='pending' WHERE delivery_state='preparing' AND NOT EXISTS(SELECT 1 FROM task_outbox o WHERE o.task_id=task_runtime.task_id AND o.revision=task_runtime.revision)",params![]).await;
    }
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(2));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        if let Err(e) = drain(&deps).await {
            tracing::warn!(%e,"任务回聊暂未完成");
        }
    }
}

async fn drain(deps: &PipelineDeps) -> Result<()> {
    let Some(engine) = &deps.reply else {
        return Ok(());
    };
    let gw = deps.llm.read().unwrap().clone();
    let Some(gw) =
        gw.filter(|g| g.role(Role::BotChat).is_some() && g.role(Role::Decision).is_some())
    else {
        return Ok(());
    };
    let mut c = db::connect(&deps.db_path).await?;
    let rows=sqlx::query("SELECT t.task_id,t.goal,t.state,r.revision,r.anchor_id,r.phase,r.final_text,r.stop_reason FROM tasks t JOIN task_runtime r USING(task_id) WHERE r.parent_id IS NULL AND r.delivery_state='pending' AND r.anchor_id IS NOT NULL ORDER BY r.updated_at LIMIT 2").fetch_all(&mut *c).await?;
    drop(c);
    for row in rows {
        let id: String = row.try_get("task_id")?;
        let revision: i64 = row.try_get("revision")?;
        let mut c = db::connect(&deps.db_path).await?;
        let claimed=c.execute("UPDATE task_runtime SET delivery_state='preparing',updated_at=unixepoch() WHERE task_id=?1 AND revision=?2 AND delivery_state='pending'",params![id,revision]).await?;
        if claimed != 1 {
            continue;
        }
        let result:Result<()>=async {
            let anchor:i64=row.try_get("anchor_id")?;
            let r=sqlx::query("SELECT * FROM messages WHERE msg_id=?").bind(anchor).fetch_one(&mut *c).await?;
            let m=MessageReceivedPayload{msg_id:anchor,chat_id:r.try_get("chat_id")?,chat_type:r.try_get("chat_type")?,sender_pid:r.try_get("sender_pid")?,text:r.try_get("text")?,at_me:r.try_get::<i64,_>("at_me")?!=0,has_image:r.try_get::<i64,_>("has_image")?!=0,reply_to:r.try_get("reply_to")?,sender_bot:false,image_urls:vec![],ts:r.try_get("ts")?};
            let cfg=*deps.ctx_cfg.read().unwrap();
            let mut snapshot=crate::context_builder::capture_reply_snapshot(&deps.db_path,&m,anchor,&cfg).await?;
            snapshot.delivery_task=Some((id.clone(),revision));
            snapshot.execution_results.push(json!({"task_id":id,"goal":row.try_get::<String,_>("goal")?,"state":row.try_get::<String,_>("phase")?,"result":row.try_get::<Option<String>,_>("final_text")?,"error":row.try_get::<Option<String>,_>("stop_reason")?}));
            let mut decision=crate::decision::decide(&deps.db_path,&gw,&deps.bus,&deps.mood,&m,deps.skill_registry.as_ref(),&snapshot).await;
            // Completion is a response event, not fresh authority to dispatch an external action.
            decision.output.action=crate::decision::DecisionAction::Reply;
            decision.output.task_control=None;
            let reply_cfg=*deps.reply_cfg.read().unwrap();
            crate::reply_engine::prepare_and_enqueue(engine,&snapshot,&gw,&deps.mood,&m,&decision.output,&cfg,&reply_cfg,&[]).await?;
            Ok(())
        }.await;
        if let Err(e) = result {
            c.execute("UPDATE task_runtime SET delivery_state='failed',stop_reason=COALESCE(stop_reason,'')||?3 WHERE task_id=?1 AND revision=?2",params![id,revision,format!("；回聊失败：{e}")]).await?;
        }
    }
    // Ready bodies survive restarts. Sending/unknown are never blindly resent.
    let mut c = db::connect(&deps.db_path).await?;
    let rows=sqlx::query("SELECT o.task_id,o.revision,o.text,t.chat_id,t.created_by_pid,r.chat_type,r.anchor_id FROM task_outbox o JOIN tasks t USING(task_id) JOIN task_runtime r USING(task_id) WHERE o.state='ready' AND o.revision=r.revision AND r.phase!='cancelled' LIMIT 8").fetch_all(&mut *c).await?;
    for row in rows {
        let id: String = row.try_get("task_id")?;
        let revision: i64 = row.try_get("revision")?;
        let chat: String = row.try_get("chat_id")?;
        let kind: String = row.try_get("chat_type")?;
        let actor: String = row.try_get("created_by_pid")?;
        let target = if kind == "private" {
            actor.strip_prefix("p_").unwrap_or("")
        } else {
            &chat
        }
        .parse()?;
        engine.enqueue(crate::reply_engine::ReplyJob {
            chat_id: chat,
            chat_type: if kind == "private" {
                crate::reply_engine::ChatType::Private
            } else {
                crate::reply_engine::ChatType::Group
            },
            target,
            anchor_msg_id: row.try_get("anchor_id")?,
            mention: kind == "group",
            mention_qq: actor.strip_prefix("p_").and_then(|s| s.parse().ok()),
            kind: crate::reply_engine::JobKind::TaskResult {
                task_id: id,
                revision,
                text: row.try_get("text")?,
            },
        });
    }
    Ok(())
}
