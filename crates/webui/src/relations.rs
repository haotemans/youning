//! 关系网数据：nodes(persons，确保含 self) + edges(relationship_edges 当前全量值)。

use crate::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};
use sqlx::Row;
use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::Ordering;
use yuantuan_core::db::SqliteExt;

/// Loaded on selection: keep the graph's initial response small.
pub async fn person_detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let mut conn = state.open_db().await.map_err(|e| err(&e.to_string()))?;
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM persons WHERE person_id=?1)",
            yuantuan_core::db::params![id],
            |r| r.try_get(0),
        )
        .await
        .map_err(|e| err(&e.to_string()))?;
    if !exists {
        return Err((StatusCode::NOT_FOUND, Json(json!({"error":"人物不存在"}))));
    }
    let profile = yuantuan_core::memory::profile(&mut conn, &id, i64::MAX)
        .await
        .map_err(|e| err(&e.to_string()))?;
    let visuals = conn.prepare("SELECT m.msg_id,m.chat_id,m.ts,i.status,i.observation,i.image_index FROM message_images i JOIN messages m ON m.msg_id=i.msg_id WHERE m.sender_pid=?1 ORDER BY m.msg_id DESC,i.image_index LIMIT 6").await.map_err(|e|err(&e.to_string()))?
        .query_map(yuantuan_core::db::params![id],|r|{
            let raw: Option<String> = r.try_get(4)?;
            let observation = raw.and_then(|s|serde_json::from_str::<yuantuan_core::vision::Observation>(&s).ok());
            Ok(json!({"source_msg_id":r.try_get::<i64,_>(0)?,"chat_id":r.try_get::<String,_>(1)?,"ts":r.try_get::<i64,_>(2)?,"status":r.try_get::<String,_>(3)?,"observation":observation,"image_index":r.try_get::<i64,_>(5)?,"identity_confirmed":false}))
        }).await.map_err(|e|err(&e.to_string()))?.collect::<sqlx::Result<Vec<_>>>().map_err(|e|err(&e.to_string()))?;
    let events = conn.prepare("SELECT from_pid,to_pid,kind,evidence,created_at FROM relationship_events WHERE from_pid=?1 OR to_pid=?1 ORDER BY id DESC LIMIT 10").await.map_err(|e|err(&e.to_string()))?
        .query_map(yuantuan_core::db::params![id],|r|Ok(json!({"from":r.try_get::<String,_>(0)?,"to":r.try_get::<String,_>(1)?,"kind":r.try_get::<String,_>(2)?,"evidence":r.try_get::<Option<String>,_>(3)?,"ts":r.try_get::<i64,_>(4)?}))).await.map_err(|e|err(&e.to_string()))?
        .collect::<sqlx::Result<Vec<_>>>().map_err(|e|err(&e.to_string()))?;
    let portrait=sqlx::query("WITH latest AS (SELECT *,ROW_NUMBER() OVER(PARTITION BY item_key ORDER BY source_end_id DESC,id DESC) AS rn FROM memory_items WHERE owner_type='person' AND owner_id=?) SELECT * FROM latest WHERE rn=1 AND status!='corrected' ORDER BY CASE WHEN item_key='summary' THEN 0 ELSE 1 END,source_end_id DESC LIMIT 24")
        .bind(&id).fetch_all(&mut *conn).await.map_err(|e|err(&e.to_string()))?;
    let portrait:Vec<Value>=portrait.iter().filter_map(|r|{let content:String=r.try_get("content").ok()?;if yuantuan_core::consolidation::is_sensitive(&content){return None}Some(json!({"id":r.try_get::<i64,_>("id").ok()?,"key":r.try_get::<String,_>("item_key").ok()?,"content":content,"kind":r.try_get::<String,_>("kind").ok()?,"status":r.try_get::<String,_>("status").ok()?,"source_chat_type":r.try_get::<String,_>("source_chat_type").ok()?,"source_chat_id":r.try_get::<String,_>("source_chat_id").ok()?,"evidence":serde_json::from_str::<Value>(&r.try_get::<String,_>("evidence").ok()?).ok()?}))}).collect();
    Ok(Json(
        json!({"portrait":portrait,"person_id":id,"profile":profile,"visual_observations":visuals,"relationship_evidence":events}),
    ))
}

pub async fn relations(
    State(state): State<AppState>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let mut conn = state
        .open_db()
        .await
        .map_err(|e| err(&format!("数据库打开失败: {e}")))?;

    let mut nodes: Vec<Value> = Vec::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT p.person_id, COALESCE(p.display_name, ''), p.first_seen, p.last_seen,
                    (SELECT i.platform_uid FROM identities i
                     WHERE i.person_id = p.person_id AND i.platform = 'qq'
                     ORDER BY i.id DESC LIMIT 1)
                 FROM persons p",
            )
            .await
            .map_err(|e| err(&format!("查询失败: {e}")))?;
        let rows = stmt
            .query_map(yuantuan_core::db::params![], |r| {
                let qq = r
                    .try_get::<Option<String>, _>(4)?
                    .filter(|uid| !uid.is_empty() && uid.bytes().all(|b| b.is_ascii_digit()));
                let avatar_url = qq
                    .as_ref()
                    .map(|uid| format!("https://q1.qlogo.cn/g?b=qq&nk={uid}&s=100"));
                Ok(json!({
                    "id": r.try_get::<String, _>(0)?,
                    "name": r.try_get::<String, _>(1)?,
                    "first_seen": r.try_get::<i64, _>(2)?,
                    "last_seen": r.try_get::<i64, _>(3)?,
                    "qq": qq,
                    "avatar_url": avatar_url,
                }))
            })
            .await
            .map_err(|e| err(&format!("查询失败: {e}")))?;
        nodes.extend(rows.filter_map(|r| r.ok()));
    }
    let own_qqs = sqlx::query_scalar::<_, String>(
        "SELECT platform_uid FROM identities WHERE person_id = 'self' AND platform = 'qq' ORDER BY id DESC",
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|e| err(&format!("查询自身身份失败: {e}")))?;

    let mut edges: Vec<Value> = Vec::new();
    {
        let mut stmt = conn
            .prepare("SELECT from_pid, to_pid, trust, familiar, updated_at FROM relationship_edges")
            .await
            .map_err(|e| err(&format!("查询失败: {e}")))?;
        let rows = stmt
            .query_map(yuantuan_core::db::params![], |r| {
                Ok(json!({
                    "from": r.try_get::<String, _>(0)?,
                    "to": r.try_get::<String, _>(1)?,
                    "trust": r.try_get::<f64, _>(2)?,
                    "familiar": r.try_get::<f64, _>(3)?,
                    "updated_at": r.try_get::<i64, _>(4)?,
                }))
            })
            .await
            .map_err(|e| err(&format!("查询失败: {e}")))?;
        edges.extend(rows.filter_map(|r| r.ok()));
    }

    Ok(Json(canonical_graph(
        nodes,
        edges,
        state.extras.self_qq.load(Ordering::Relaxed),
        own_qqs,
    )))
}

/// Project legacy bot aliases onto self. Stored messages and relationship rows
/// remain intact; equal display names never establish identity.
fn canonical_graph(
    mut nodes: Vec<Value>,
    edges: Vec<Value>,
    live_qq: u64,
    mut own_qqs: Vec<String>,
) -> Value {
    if live_qq != 0 {
        own_qqs.insert(0, live_qq.to_string());
    }
    own_qqs.retain(|q| !q.is_empty() && q.bytes().all(|b| b.is_ascii_digit()));
    let mut aliases: HashSet<String> = own_qqs.iter().map(|q| format!("p_{q}")).collect();
    aliases.insert("self".into());
    for node in &nodes {
        if node["qq"]
            .as_str()
            .is_some_and(|q| own_qqs.iter().any(|own| own == q))
        {
            if let Some(id) = node["id"].as_str() {
                aliases.insert(id.to_owned());
            }
        }
    }
    let mut me = nodes
        .iter()
        .find(|n| n["id"] == "self")
        .cloned()
        .unwrap_or_else(|| json!({"id":"self", "name":"云团"}));
    nodes.retain(|node| {
        if !node["id"].as_str().is_some_and(|id| aliases.contains(id)) {
            return true;
        }
        for (field, earliest) in [("first_seen", true), ("last_seen", false)] {
            if let Some(value) = node[field].as_i64() {
                let merged = me[field].as_i64().map_or(value, |old| {
                    if earliest {
                        old.min(value)
                    } else {
                        old.max(value)
                    }
                });
                me[field] = json!(merged);
            }
        }
        false
    });
    if let Some(qq) = own_qqs.first() {
        me["qq"] = json!(qq);
        me["avatar_url"] = json!(format!("https://q1.qlogo.cn/g?b=qq&nk={qq}&s=100"));
    }
    nodes.insert(0, me);

    let mut merged: BTreeMap<(String, String), (bool, Value)> = BTreeMap::new();
    for mut edge in edges {
        let canonical = edge["from"] == "self" || edge["to"] == "self";
        for field in ["from", "to"] {
            if edge[field].as_str().is_some_and(|id| aliases.contains(id)) {
                edge[field] = json!("self");
            }
        }
        if edge["from"] == "self" && edge["to"] == "self" {
            continue;
        }
        let pair = (
            edge["from"].as_str().unwrap_or("").to_owned(),
            edge["to"].as_str().unwrap_or("").to_owned(),
        );
        let replace = merged.get(&pair).is_none_or(|(old_canonical, old)| {
            (edge["updated_at"].as_i64().unwrap_or(0), canonical)
                > (old["updated_at"].as_i64().unwrap_or(0), *old_canonical)
        });
        if replace {
            merged.insert(pair, (canonical, edge));
        }
    }
    let edges: Vec<_> = merged.into_values().map(|(_, edge)| edge).collect();
    json!({"nodes":nodes, "edges":edges})
}

fn err(msg: &str) -> (StatusCode, Json<Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": msg })),
    )
}
