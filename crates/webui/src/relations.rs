//! 关系网数据：nodes(persons，确保含 self) + edges(relationship_edges 当前全量值)。

use crate::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};
use sqlx::Row;
use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::Ordering;
use yuantuan_core::db::SqliteExt;

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
