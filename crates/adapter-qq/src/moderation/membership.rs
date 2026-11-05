use super::{role, Transport};
use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};
use sqlx::Row;
use std::path::Path;
use yuantuan_core::{
    db::{self, params, SqliteExt},
    membership::{self as queue, Request},
};
use yuantuan_plugin_api::{admission_rules, MembershipAction as Action, ToolCtx};

fn number(s: &str) -> Result<u64> {
    s.parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
        .context("QQ或群号无效")
}
fn scope(group: &str, allowed: Option<&[String]>) -> Result<()> {
    ensure!(
        allowed.is_none_or(|a| a.iter().any(|s| s == group)),
        "申请所在会话不在插件授权范围内"
    );
    Ok(())
}
async fn authorize(
    client: &dyn Transport,
    ctx: &ToolCtx,
    r: &Request,
    allowed: Option<&[String]>,
    owner: Option<u64>,
) -> Result<()> {
    let actor = ctx
        .sender_pid
        .strip_prefix("p_")
        .map(number)
        .transpose()?
        .context("申请操作员身份无效")?;
    ensure!(actor != client.bot_qq(), "机器人不能发起手动审核指令");
    if r.request_type == "friend" {
        ensure!(
            ctx.chat_type == "private" && Some(actor) == owner,
            "好友管理仅限超级管理员私聊操作"
        );
        scope(&ctx.chat_id, allowed)?;
    } else {
        scope(&r.group_id, allowed)?;
        if ctx.chat_type == "private" {
            ensure!(Some(actor) == owner, "私聊审核入群申请仅限超级管理员");
        } else {
            ensure!(
                ctx.chat_type == "group" && ctx.chat_id == r.group_id,
                "不能跨群审核申请"
            );
        }
        if let Some(owner) = owner {
            ensure!(actor == owner, "仅云团超级管理员可以审核申请");
        } else {
            ensure!(
                role(client, number(&r.group_id)?, actor).await? != "member",
                "仅本群群主或管理员可以审核申请"
            );
        }
        ensure!(
            role(client, number(&r.group_id)?, client.bot_qq()).await? != "member",
            "云团需要本群管理员权限才能审核入群申请"
        );
    }
    Ok(())
}
async fn auto_authorize(
    client: &dyn Transport,
    ctx: &ToolCtx,
    r: &Request,
    policy: &Value,
    allowed: Option<&[String]>,
) -> Result<yuantuan_plugin_api::AdmissionRule> {
    ensure!(
        ctx.sender_pid == "self" && ctx.chat_type == "group" && ctx.chat_id == r.group_id,
        "自动审核上下文不匹配"
    );
    ensure!(
        policy["auto_group_review"].as_bool().unwrap_or(false),
        "未开启自动入群审核"
    );
    ensure!(
        r.request_type == "group" && r.sub_type == "add",
        "邀请云团进群不能自动同意"
    );
    scope(&r.group_id, allowed)?;
    let rule = admission_rules(policy)?
        .remove(&r.group_id)
        .context("本群尚未配置入群规则")?;
    ensure!(
        role(client, number(&r.group_id)?, client.bot_qq()).await? != "member",
        "云团没有本群管理员权限"
    );
    Ok(rule)
}
async fn request(path: &Path, id: i64, bot: u64) -> Result<Request> {
    let r = queue::load(path, id).await?;
    ensure!(r.bot_qq == bot.to_string(), "申请不属于当前登录QQ");
    ensure!(
        queue::now() - r.created_at <= 7 * 86400,
        "申请已超过7天，请在QQ客户端核实"
    );
    Ok(r)
}
async fn info(client: &dyn Transport, r: &Request) -> Result<Value> {
    let value = client
        .action(
            "get_stranger_info",
            json!({"user_id":number(&r.user_id)?,"no_cache":true}),
        )
        .await?;
    ensure!(
        value["user_id"].as_u64() == Some(number(&r.user_id)?),
        "申请人资料返回身份不匹配"
    );
    Ok(
        json!({"user_id":r.user_id,"nickname":value["nickname"].as_str().unwrap_or(""),"qq_level":value["qqLevel"].as_u64().filter(|n|*n<=256)}),
    )
}
async fn decide(
    client: &dyn Transport,
    path: &Path,
    r: &Request,
    approve: bool,
    reason: &str,
    remark: &str,
    automatic: bool,
) -> Result<Value> {
    ensure!(
        reason.chars().count() <= 300 && remark.chars().count() <= 100,
        "审核理由或备注过长"
    );
    let mut conn = db::connect(path).await?;
    let claimed=conn.execute("UPDATE membership_requests SET status='processing',note=?2,updated_at=?3 WHERE id=?1 AND (status='assessing' OR (?4=0 AND status='pending'))",params![r.id,reason,queue::now(),automatic]).await?>0;
    ensure!(claimed, "申请已处理或结果待确认，不重复提交");
    drop(conn);
    let (action, args) = if r.request_type == "friend" {
        (
            "set_friend_add_request",
            json!({"flag":r.flag,"approve":approve,"remark":remark}),
        )
    } else {
        (
            "set_group_add_request",
            json!({"flag":r.flag,"sub_type":r.sub_type,"approve":approve,"reason":if approve{""}else{reason}}),
        )
    };
    match client.action(action, args).await {
        Ok(_) => {
            queue::finish(
                path,
                r.id,
                "processing",
                if approve { "approved" } else { "rejected" },
                reason,
                None,
            )
            .await?;
            Ok(json!({"id":r.id,"status":if approve{"approved"}else{"rejected"},"submitted":true}))
        }
        Err(e) => {
            queue::finish(
                path,
                r.id,
                "processing",
                "uncertain",
                "接口结果未确认，请在QQ客户端核实；不会自动重试",
                None,
            )
            .await?;
            Err(e.context("申请审核结果未确认，不要盲目重复提交"))
        }
    }
}

pub(super) async fn execute(
    client: &dyn Transport,
    path: &Path,
    ctx: &ToolCtx,
    action: Action,
    policy: &Value,
    allowed: Option<&[String]>,
    owner: Option<u64>,
) -> Result<Value> {
    let bot = client.bot_qq();
    ensure!(bot > 0, "QQ登录身份未就绪");
    if let Action::AutoPending = &action {
        ensure!(ctx.sender_pid == "self", "自动审核身份无效");
        if !policy["auto_group_review"].as_bool().unwrap_or(false) {
            return Ok(json!([]));
        }
        let groups: Vec<String> = admission_rules(policy)?
            .into_keys()
            .filter(|g| scope(g, allowed).is_ok())
            .collect();
        if groups.is_empty() {
            return Ok(json!([]));
        }
        let mut conn = db::connect(path).await?;
        let count:i64=sqlx::query_scalar("SELECT COALESCE(sum(auto_attempts),0) FROM membership_requests WHERE bot_qq=?1 AND reviewed_at>=?2").bind(bot.to_string()).bind(queue::now()-86400).fetch_one(&mut *conn).await?;
        if count >= policy["max_auto_reviews_per_day"].as_i64().unwrap_or(20) {
            return Ok(json!([]));
        }
        let mut q = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "SELECT * FROM membership_requests WHERE bot_qq=",
        );
        q.push_bind(bot.to_string()).push(" AND request_type='group' AND sub_type='add' AND created_at>=").push_bind(queue::now()-7*86400)
            .push(" AND auto_attempts<2 AND ((status='pending' AND reviewed_at=0) OR (status='assessing' AND updated_at<").push_bind(queue::now()-180).push(")) AND group_id IN (");
        let mut ids = q.separated(",");
        for group in groups {
            ids.push_bind(group);
        }
        ids.push_unseparated(")");
        q.push(" ORDER BY id LIMIT 5");
        let rows = q.build().fetch_all(&mut *conn).await?;
        return Ok(json!(rows
            .iter()
            .map(|r| json!({"id":r.get::<i64,_>("id"),"group_id":r.get::<String,_>("group_id")}))
            .collect::<Vec<_>>()));
    }
    if let Action::List { kind } = &action {
        ensure!(["friend", "group"].contains(&kind.as_str()), "申请类型无效");
        let sample = Request {
            id: 0,
            bot_qq: bot.to_string(),
            request_type: kind.clone(),
            sub_type: String::new(),
            group_id: ctx.chat_id.clone(),
            user_id: ctx.chat_id.clone(),
            flag: String::new(),
            comment: String::new(),
            status: String::new(),
            note: String::new(),
            assessment: None,
            reviewed_at: 0,
            created_at: 0,
            updated_at: 0,
        };
        authorize(client, ctx, &sample, allowed, owner).await?;
        let mut conn = db::connect(path).await?;
        let rows=sqlx::query("SELECT * FROM membership_requests WHERE bot_qq=?1 AND request_type=?2 AND (?3='friend' OR group_id=?4) AND status IN ('pending','assessing','processing','uncertain') ORDER BY id DESC LIMIT 20").bind(bot.to_string()).bind(kind).bind(kind).bind(&ctx.chat_id).fetch_all(&mut *conn).await?;
        return Ok(json!(rows
            .iter()
            .map(|row| {
                let mut request = Request::from_row(row)?;
                request.comment = request.comment.chars().take(100).collect();
                request.note = request.note.chars().take(100).collect();
                request.assessment = None;
                Ok(request)
            })
            .collect::<sqlx::Result<Vec<_>>>()?));
    }
    if let Action::AddFriend { user_id, remark } = &action {
        ensure!(
            *user_id > 0
                && ctx.chat_type == "private"
                && ctx.sender_pid == format!("p_{}", owner.context("请先配置超级管理员QQ")?),
            "加好友仅限超级管理员私聊操作"
        );
        scope(&ctx.chat_id, allowed)?;
        let friends = client
            .action("get_friend_list", json!({"no_cache":true}))
            .await?;
        ensure!(friends.is_array(), "好友列表格式无效");
        if friends
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["user_id"].as_u64() == Some(*user_id))
        {
            return Ok(json!({"status":"already_friend","user_id":user_id}));
        }
        let mut conn = db::connect(path).await?;
        let id:Option<i64>=sqlx::query_scalar("SELECT id FROM membership_requests WHERE bot_qq=?1 AND request_type='friend' AND user_id=?2 AND status='pending' ORDER BY id DESC LIMIT 1").bind(bot.to_string()).bind(user_id.to_string()).fetch_optional(&mut *conn).await?;
        drop(conn);
        let r = request(
            path,
            id.context(
                "尚无对方好友申请；当前NapCat公开接口不能主动发送加好友请求，请让对方先添加云团",
            )?,
            bot,
        )
        .await?;
        return decide(
            client,
            path,
            &r,
            true,
            "超级管理员同意好友申请",
            remark,
            false,
        )
        .await;
    }
    let id = match &action {
        Action::Inspect { id }
        | Action::Decide { id, .. }
        | Action::AutoInspect { id }
        | Action::AutoDecide { id, .. }
        | Action::Defer { id, .. } => *id,
        _ => unreachable!(),
    };
    let r = request(path, id, bot).await?;
    match action {
        Action::Inspect { .. } => {
            authorize(client, ctx, &r, allowed, owner).await?;
            Ok(json!({"request":r,"applicant":info(client,&r).await?}))
        }
        Action::Decide {
            approve,
            reason,
            remark,
            ..
        } => {
            authorize(client, ctx, &r, allowed, owner).await?;
            decide(client, path, &r, approve, &reason, &remark, false).await
        }
        Action::AutoInspect { .. } => {
            let rule = auto_authorize(client, ctx, &r, policy, allowed).await?;
            let mut conn = db::connect(path).await?;
            let changed=conn.execute("UPDATE membership_requests SET status='assessing',updated_at=?2,reviewed_at=?2,auto_attempts=auto_attempts+1 WHERE id=?1 AND auto_attempts<2 AND ((status='pending' AND reviewed_at=0) OR (status='assessing' AND updated_at<?2-180)) AND (SELECT COALESCE(sum(auto_attempts),0) FROM membership_requests WHERE bot_qq=?3 AND reviewed_at>=?2-86400)<?4",params![id,queue::now(),bot.to_string(),policy["max_auto_reviews_per_day"].as_i64().unwrap_or(20)]).await?>0;
            ensure!(changed, "申请已被领取或处理");
            drop(conn);
            ensure!(
                !yuantuan_core::consolidation::is_sensitive(&r.comment),
                "申请理由包含敏感内容，保留人工审核"
            );
            let applicant = info(client, &r).await?;
            let mut conn = db::connect(path).await?;
            let assessment = json!({"applicant":applicant,"rule":rule});
            conn.execute(
                "UPDATE membership_requests SET assessment=?2 WHERE id=?1 AND status='assessing'",
                params![id, assessment.to_string()],
            )
            .await?;
            let topics:Vec<String>=sqlx::query_scalar::<_,String>("SELECT lm.content FROM long_memories lm
                WHERE lm.owner_type='chat' AND lm.owner_id=?1 AND lm.source_chat_id=?1
                AND EXISTS(SELECT 1 FROM messages m WHERE m.msg_id=lm.source_msg_id AND m.chat_id=?1 AND m.chat_type='group')
                AND (lm.source_end_msg_id IS NULL OR (lm.source_end_msg_id>=lm.source_msg_id AND EXISTS(SELECT 1 FROM messages m WHERE m.msg_id=lm.source_end_msg_id AND m.chat_id=?1 AND m.chat_type='group')))
                AND NOT EXISTS(SELECT 1 FROM messages m WHERE m.chat_id=?1 AND m.chat_type<>'group' AND m.msg_id BETWEEN lm.source_msg_id AND COALESCE(lm.source_end_msg_id,lm.source_msg_id))
                ORDER BY lm.updated_at DESC LIMIT 3").bind(&r.group_id).fetch_all(&mut *conn).await?.into_iter().filter(|s|!yuantuan_core::consolidation::is_sensitive(s)).map(|s|s.chars().take(300).collect()).collect();
            Ok(json!({"request":r,"applicant":applicant,"rule":rule,"group_topics":topics}))
        }
        Action::Defer { reason, .. } => {
            auto_authorize(client, ctx, &r, policy, allowed).await?;
            ensure!(
                reason.chars().count() <= 300
                    && !yuantuan_core::consolidation::is_sensitive(&reason),
                "待审说明无效"
            );
            queue::finish(path, id, "assessing", "pending", &reason, None).await?;
            Ok(json!({"status":"pending"}))
        }
        Action::AutoDecide {
            approve,
            reason,
            confidence,
            rule_quote,
            ..
        } => {
            let rule = auto_authorize(client, ctx, &r, policy, allowed).await?;
            ensure!(r.status == "assessing", "申请不在待评估状态");
            ensure!(
                confidence.is_finite() && (0.9..=1.0).contains(&confidence),
                "判断置信度不足"
            );
            ensure!(!reason.trim().is_empty(), "审核依据不能为空");
            ensure!(
                !rule_quote.trim().is_empty() && rule.rules.contains(&rule_quote),
                "没有配置群规的原文依据"
            );
            let mut assessment: Value =
                serde_json::from_str(r.assessment.as_deref().context("缺少申请人资料依据")?)?;
            ensure!(assessment["rule"] == json!(rule), "审核期间群规则发生变化");
            if rule.min_qq_level > 0 && approve {
                ensure!(
                    assessment["applicant"]["qq_level"]
                        .as_u64()
                        .is_some_and(|n| n >= rule.min_qq_level as u64),
                    "QQ等级未达到门槛或未读取到"
                );
            }
            ensure!(
                !yuantuan_core::consolidation::is_sensitive(&reason),
                "审核理由包含敏感内容"
            );
            assessment["judgment"] = json!({"approve":approve,"confidence":confidence,"reason":reason,"rule_quote":rule_quote});
            let mut conn = db::connect(path).await?;
            let saved=conn.execute("UPDATE membership_requests SET assessment=?2 WHERE id=?1 AND status='assessing'",params![id,assessment.to_string()]).await?>0;
            ensure!(saved, "申请已由其他操作完成");
            drop(conn);
            decide(client, path, &r, approve, &reason, "", true).await
        }
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests;
