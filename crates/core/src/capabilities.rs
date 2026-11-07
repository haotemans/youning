//! Persistent capability switches and invocation receipts; plugin permission checks still apply.
use crate::{
    db::{self, params, SqliteExt},
    skills::Skill,
    tools::{ToolCtx, ToolOutput},
};
use anyhow::{ensure, Result};
use serde_json::{json, Value};
use sqlx::Row;
use std::{path::Path, sync::Arc};

pub async fn register(
    path: &Path,
    name: &str,
    kind: &str,
    description: &str,
    schema: &Value,
) -> Result<()> {
    let mut c = db::connect(path).await?;
    c.execute("INSERT INTO capability_catalog(name,kind,description,schema_json,updated_at) VALUES(?1,?2,?3,?4,unixepoch()) ON CONFLICT(name,kind) DO UPDATE SET description=excluded.description,schema_json=excluded.schema_json,updated_at=excluded.updated_at WHERE description!=excluded.description OR schema_json!=excluded.schema_json",
        params![name,kind,description,schema.to_string()]).await?;
    Ok(())
}
pub async fn allowed(path: &Path, name: &str, kind: &str, ctx: &ToolCtx) -> Result<bool> {
    let mut c = db::connect(path).await?;
    let row = sqlx::query("SELECT enabled,scopes FROM capability_catalog WHERE name=? AND kind=?")
        .bind(name)
        .bind(kind)
        .fetch_optional(&mut *c)
        .await?;
    let Some(row) = row else { return Ok(true) };
    let scopes: Vec<String> = serde_json::from_str(&row.try_get::<String, _>("scopes")?)?;
    Ok(row.try_get::<bool, _>("enabled")?
        && (scopes.is_empty() || scopes.contains(&format!("{}:{}", ctx.chat_type, ctx.chat_id))))
}
pub fn validate(schema: &Value, args: &Value) -> Result<()> {
    if let Some(kind) = schema["type"].as_str() {
        ensure!(
            match kind {
                "object" => args.is_object(),
                "array" => args.is_array(),
                "string" => args.is_string(),
                "integer" => args.is_i64() || args.is_u64(),
                "number" => args.is_number(),
                "boolean" => args.is_boolean(),
                "null" => args.is_null(),
                _ => false,
            },
            "工具参数类型不匹配"
        );
    }
    if let Some(values) = schema["enum"].as_array() {
        ensure!(values.contains(args), "参数不在允许值中");
    }
    if let Some(required) = schema["required"].as_array() {
        for k in required {
            ensure!(args.get(k.as_str().unwrap_or("")).is_some(), "缺少必填参数");
        }
    }
    if let Some(properties) = schema["properties"].as_object() {
        for (k, s) in properties {
            if let Some(v) = args.get(k) {
                validate(s, v)?;
            }
        }
    }
    if let Some(values) = args.as_array() {
        if schema.get("items").is_some() {
            for v in values {
                validate(&schema["items"], v)?;
            }
        }
    }
    if let Some(v) = args.as_f64() {
        ensure!(
            schema["minimum"].as_f64().is_none_or(|n| v >= n)
                && schema["maximum"].as_f64().is_none_or(|n| v <= n),
            "参数超出范围"
        );
    }
    Ok(())
}
pub async fn check_tool(
    path: &Path,
    name: &str,
    ctx: &ToolCtx,
    schema: &Value,
    args: &Value,
) -> Result<()> {
    ensure!(
        allowed(path, name, "tool", ctx).await?,
        "工具已停用或不在会话范围内"
    );
    validate(schema, args)
}
pub async fn invoke(
    path: &Path,
    skill: &Arc<dyn Skill>,
    ctx: &ToolCtx,
    anchor: Option<i64>,
    slots: Value,
) -> Result<ToolOutput> {
    let def = skill.def();
    register(
        path,
        def.name,
        "skill",
        def.description,
        &json!({"type":"object"}),
    )
    .await?;
    ensure!(
        slots.is_object()
            && skill.available_in_chat(&ctx.chat_id)
            && allowed(path, def.name, "skill", ctx).await?,
        "技能已停用、范围不符或参数无效"
    );
    let id = {
        let mut c = db::connect(path).await?;
        sqlx::query_scalar::<_,i64>("INSERT INTO capability_invocations(name,kind,chat_id,chat_type,actor,anchor_id,state,started_at) VALUES(?1,'skill',?2,?3,?4,?5,'started',unixepoch()) RETURNING id")
            .bind(def.name).bind(&ctx.chat_id).bind(&ctx.chat_type).bind(&ctx.sender_pid).bind(anchor).fetch_one(&mut *c).await?
    };
    let result = skill.invoke(ctx, slots).await;
    let summary = match &result {
        Ok(out) => out.summary.clone(),
        Err(_) => "技能执行失败，外部动作是否生效需核实".into(),
    };
    let mut c = db::connect(path).await?;
    c.execute(
        "UPDATE capability_invocations SET state=?2,summary=?3,finished_at=unixepoch() WHERE id=?1",
        params![
            id,
            if result.is_ok() {
                "succeeded"
            } else {
                "unknown"
            },
            summary.chars().take(1000).collect::<String>()
        ],
    )
    .await?;
    result
}
