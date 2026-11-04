//! Read-only memory browsing, unified people, conversation filters and source messages.
use crate::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::{QueryBuilder, Row, Sqlite};

type ApiResult = Result<Json<Value>, (StatusCode, Json<Value>)>;
fn error(status: StatusCode, message: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    (status, Json(json!({"error":message.to_string()})))
}
fn db_error(e: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    error(StatusCode::INTERNAL_SERVER_ERROR, e)
}

#[derive(Default, Deserialize)]
pub struct MemQuery {
    owner_type: Option<String>,
    owner_id: Option<String>,
    person_id: Option<String>,
    chat_id: Option<String>,
    chat_type: Option<String>,
    search: Option<String>,
    limit: Option<i64>,
    page: Option<i64>,
    page_size: Option<i64>,
}
impl MemQuery {
    fn validate(&self) -> Result<(), (StatusCode, Json<Value>)> {
        if self
            .owner_type
            .as_deref()
            .is_some_and(|v| !["person", "chat", "self"].contains(&v))
            || self
                .chat_type
                .as_deref()
                .is_some_and(|v| !["group", "private"].contains(&v))
            || self
                .search
                .as_ref()
                .is_some_and(|s| s.chars().count() > 100)
            || self.page.is_some_and(|p| p < 1 || p > i64::MAX / 96)
            || self.page_size.is_some_and(|p| !(1..=96).contains(&p))
            || [&self.owner_id, &self.person_id, &self.chat_id]
                .into_iter()
                .flatten()
                .any(|s| s.len() > 160)
        {
            return Err(error(StatusCode::BAD_REQUEST, "查询参数无效"));
        }
        Ok(())
    }
}

// Old person summaries have no conversation column. Never infer a source from nearby messages.
fn cte(summary: bool) -> String {
    let records = if summary {
        "SELECT id,owner_type,owner_id,summary AS content,'consolidation' AS source,created_at,created_at AS updated_at,
            date,period,msg_id_start AS source_msg_id,msg_id_end AS source_end_msg_id,
            CASE WHEN owner_type='chat' THEN owner_id END AS chat_id FROM summaries"
    } else {
        "SELECT id,owner_type,owner_id,content,source,created_at,updated_at,NULL AS date,NULL AS period,
            source_msg_id,source_end_msg_id,CASE WHEN owner_type='chat' THEN owner_id ELSE source_chat_id END AS chat_id FROM long_memories"
    };
    format!("WITH records AS ({records}), enriched AS (
        SELECT r.*,COALESCE(NULLIF(p.display_name,''),CASE WHEN r.owner_type='self' THEN '云团' END) AS owner_name,
        (SELECT platform_uid FROM identities WHERE person_id=p.person_id AND platform='qq' ORDER BY id DESC LIMIT 1) AS qq,
        (SELECT chat_type FROM messages WHERE msg_id=r.source_msg_id AND chat_id=r.chat_id) AS chat_type,
        (SELECT card FROM member_profiles WHERE person_id=r.owner_id AND chat_id=r.chat_id) AS card
        FROM records r LEFT JOIN persons p ON p.person_id=CASE WHEN r.owner_type='self' THEN 'self' ELSE r.owner_id END)")
}

fn filtered(summary: bool, q: &MemQuery, count: bool) -> QueryBuilder<'static, Sqlite> {
    let mut b = QueryBuilder::new(format!(
        "{} SELECT {} FROM enriched e WHERE trim(COALESCE(content,''))!=''
        AND lower(trim(content)) NOT IN ('null','undefined')",
        cte(summary),
        if count { "COUNT(*)" } else { "*" }
    ));
    for (column, value) in [
        ("owner_type", &q.owner_type),
        ("owner_id", &q.owner_id),
        ("chat_id", &q.chat_id),
        ("chat_type", &q.chat_type),
    ] {
        if let Some(value) = value.as_ref().filter(|s| !s.trim().is_empty()) {
            b.push(format!(" AND {column}="))
                .push_bind(value.trim().to_owned());
        }
    }
    if let Some(id) = &q.person_id {
        b.push(" AND ((owner_type='person' AND owner_id=")
            .push_bind(id.clone())
            .push(") OR (owner_type='self' AND ")
            .push_bind(id.clone())
            .push("='self'))");
    }
    if let Some(term) = q
        .search
        .as_ref()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
    {
        b.push(" AND (instr(lower(content),").push_bind(term.clone()).push(")>0 OR instr(lower(COALESCE(owner_name,'')),")
            .push_bind(term.clone()).push(")>0 OR instr(owner_id,").push_bind(term.clone()).push(")>0 OR instr(COALESCE(qq,''),")
            .push_bind(term.clone()).push(")>0 OR EXISTS(SELECT 1 FROM member_profiles mp WHERE mp.person_id=e.owner_id AND instr(lower(COALESCE(mp.card,'')),")
            .push_bind(term).push(")>0))");
    }
    b
}

fn identity(id: &str, name: Option<String>, qq: Option<String>, kind: &str) -> Value {
    let qq = qq.filter(|s| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit()));
    let name = name.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| {
        if kind == "chat" {
            format!("会话 {id}")
        } else {
            "未知成员".into()
        }
    });
    json!({"id":id,"name":name,"qq":qq,"avatar_url":qq.as_ref().map(|q|format!("https://q1.qlogo.cn/g?b=qq&nk={q}&s=100")),"kind":kind})
}
fn conversation(id: Option<String>, kind: Option<String>) -> Option<Value> {
    id.map(|id| {
        let label = match kind.as_deref() {
            Some("group") => format!("群聊 {id}"),
            Some("private") => format!("私聊 {id}"),
            _ => format!("会话 {id}"),
        };
        json!({"id":id,"chat_type":kind,"label":label})
    })
}

async fn list(state: AppState, q: MemQuery, summary: bool) -> ApiResult {
    q.validate()?;
    let mut conn = state.open_db().await.map_err(db_error)?;
    let mut tx = sqlx::Connection::begin(&mut *conn)
        .await
        .map_err(db_error)?;
    let memory_count: i64 = filtered(false, &q, true)
        .build_query_scalar()
        .fetch_one(&mut *tx)
        .await
        .map_err(db_error)?;
    let summary_count: i64 = filtered(true, &q, true)
        .build_query_scalar()
        .fetch_one(&mut *tx)
        .await
        .map_err(db_error)?;
    let total = if summary { summary_count } else { memory_count };
    let paginated = q.page.is_some() || q.page_size.is_some();
    let size = if paginated {
        q.page_size.unwrap_or(24)
    } else {
        q.limit.unwrap_or(100).clamp(1, 500)
    };
    let page = q.page.unwrap_or(1).min(((total - 1).max(0) / size) + 1);
    let mut b = filtered(summary, &q, false);
    b.push(" ORDER BY updated_at DESC,id DESC LIMIT ")
        .push_bind(size)
        .push(" OFFSET ")
        .push_bind((page - 1) * size);
    let rows = b.build().fetch_all(&mut *tx).await.map_err(db_error)?;
    let items=rows.iter().map(|r| -> sqlx::Result<Value> {
        let owner_id:String=r.try_get("owner_id")?;
        let owner_type:String=r.try_get("owner_type")?;
        let content:String=r.try_get("content")?;
        let mut item=json!({"id":r.try_get::<i64,_>("id")?,"owner_type":owner_type,"owner_id":owner_id,
            "owner":identity(&owner_id,r.try_get("owner_name")?,r.try_get("qq")?,&owner_type),
            "content":content,"source":r.try_get::<String,_>("source")?,"created_at":r.try_get::<i64,_>("created_at")?,
            "updated_at":r.try_get::<i64,_>("updated_at")?,"conversation":conversation(r.try_get("chat_id")?,r.try_get("chat_type")?),
            "source_msg_id":r.try_get::<Option<i64>,_>("source_msg_id")?,"source_end_msg_id":r.try_get::<Option<i64>,_>("source_end_msg_id")?,
            "group_card":r.try_get::<Option<String>,_>("card")?});
        if summary {item["summary"]=json!(content);item["date"]=json!(r.try_get::<String,_>("date")?);
            item["period"]=json!(r.try_get::<String,_>("period")?);
            item["msg_id_start"]=item["source_msg_id"].clone();item["msg_id_end"]=item["source_end_msg_id"].clone();}
        Ok(item)
    }).collect::<sqlx::Result<Vec<_>>>().map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    let mut data = json!({"total":total,"page":page,"page_size":size,"counts":{"memories":memory_count,"summaries":summary_count}});
    data[if summary { "summaries" } else { "memories" }] = json!(items);
    Ok(Json(data))
}
pub async fn memories(State(state): State<AppState>, Query(q): Query<MemQuery>) -> ApiResult {
    list(state, q, false).await
}
pub async fn summaries(State(state): State<AppState>, Query(q): Query<MemQuery>) -> ApiResult {
    list(state, q, true).await
}

fn people_filtered(q: &MemQuery, count: bool) -> QueryBuilder<'static, Sqlite> {
    let projection = if count {
        "COUNT(*)"
    } else {
        "p.person_id,p.display_name,p.last_seen,
        (SELECT platform_uid FROM identities WHERE person_id=p.person_id AND platform='qq' ORDER BY id DESC LIMIT 1) AS qq"
    };
    let mut b =
        QueryBuilder::<Sqlite>::new(format!("SELECT {projection} FROM persons p WHERE 1=1"));
    if let Some(chat) = &q.chat_id {
        b.push(
            " AND EXISTS(SELECT 1 FROM messages m WHERE m.sender_pid=p.person_id AND m.chat_id=",
        )
        .push_bind(chat.clone());
        if let Some(kind) = &q.chat_type {
            b.push(" AND m.chat_type=").push_bind(kind.clone());
        }
        b.push(")");
    }
    if let Some(term) = q
        .search
        .as_ref()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
    {
        b.push(" AND (instr(lower(COALESCE(p.display_name,'')),").push_bind(term.clone()).push(")>0 OR instr(p.person_id,").push_bind(term.clone())
            .push(")>0 OR EXISTS(SELECT 1 FROM identities i WHERE i.person_id=p.person_id AND instr(i.platform_uid,").push_bind(term.clone()).push(")>0)")
            .push(" OR EXISTS(SELECT 1 FROM member_profiles mp WHERE mp.person_id=p.person_id AND instr(lower(COALESCE(mp.card,'')),").push_bind(term.clone()).push(")>0)")
            .push(" OR EXISTS(SELECT 1 FROM long_memories lm WHERE lm.owner_type='person' AND lm.owner_id=p.person_id AND instr(lower(COALESCE(lm.content,'')),").push_bind(term.clone()).push(")>0)")
            .push(" OR EXISTS(SELECT 1 FROM summaries s WHERE s.owner_type='person' AND s.owner_id=p.person_id AND instr(lower(COALESCE(s.summary,'')),").push_bind(term.clone()).push(")>0)")
            .push(" OR EXISTS(SELECT 1 FROM person_profile_facts pf WHERE pf.person_id=p.person_id AND instr(lower(pf.content),").push_bind(term).push(")>0))");
    }
    b
}

pub async fn people(State(state): State<AppState>, Query(q): Query<MemQuery>) -> ApiResult {
    q.validate()?;
    let mut conn = state.open_db().await.map_err(db_error)?;
    let mut tx = sqlx::Connection::begin(&mut *conn)
        .await
        .map_err(db_error)?;
    let total: i64 = people_filtered(&q, true)
        .build_query_scalar()
        .fetch_one(&mut *tx)
        .await
        .map_err(db_error)?;
    let size = q.page_size.unwrap_or(24);
    let page = q.page.unwrap_or(1).min((total - 1).max(0) / size + 1);
    let mut b = people_filtered(&q, false);
    b.push(" ORDER BY p.last_seen DESC,p.person_id LIMIT ")
        .push_bind(size)
        .push(" OFFSET ")
        .push_bind((page - 1) * size);
    let rows = b.build().fetch_all(&mut *tx).await.map_err(db_error)?;
    let ids = rows
        .iter()
        .map(|r| r.try_get::<String, _>("person_id"))
        .collect::<sqlx::Result<Vec<_>>>()
        .map_err(db_error)?;
    let mut portraits = yuantuan_core::memory::profiles(&mut tx, &ids, i64::MAX)
        .await
        .map_err(db_error)?;
    let items = rows
        .iter()
        .take(size as usize)
        .map(|r| -> sqlx::Result<Value> {
            let id: String = r.try_get("person_id")?;
            let mut person = identity(&id, r.try_get("display_name")?, r.try_get("qq")?, "person");
            let facts = portraits.remove(&id).unwrap_or_default();
            person["profile_updated_at"] = json!(facts.iter().map(|f| f.updated_at).max());
            person["profile"] = json!(facts);
            Ok(person)
        })
        .collect::<sqlx::Result<Vec<_>>>()
        .map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    Ok(Json(
        json!({"people":items,"page":page,"total":total,"has_more":page*size<total}),
    ))
}

pub async fn conversations(State(state): State<AppState>, Query(q): Query<MemQuery>) -> ApiResult {
    q.validate()?;
    let mut conn = state.open_db().await.map_err(db_error)?;
    let mut b=QueryBuilder::<Sqlite>::new("SELECT chat_id,chat_type,MAX(ts) AS last_seen FROM messages WHERE chat_type IN ('group','private')");
    if let Some(person) = q.person_id {
        b.push(" AND sender_pid=").push_bind(person);
    }
    b.push(" GROUP BY chat_id,chat_type ORDER BY last_seen DESC,chat_id LIMIT 200");
    let rows = b.build().fetch_all(&mut *conn).await.map_err(db_error)?;
    let items = rows
        .iter()
        .map(|r| -> sqlx::Result<Value> {
            Ok(conversation(Some(r.try_get("chat_id")?), Some(r.try_get("chat_type")?)).unwrap())
        })
        .collect::<sqlx::Result<Vec<_>>>()
        .map_err(db_error)?;
    Ok(Json(json!({"conversations":items})))
}

#[derive(Deserialize)]
pub struct SourceQuery {
    kind: Option<String>,
}
pub async fn source(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(q): Query<SourceQuery>,
) -> ApiResult {
    let summary = match q.kind.as_deref().unwrap_or("memory") {
        "memory" => false,
        "summary" => true,
        _ => return Err(error(StatusCode::BAD_REQUEST, "来源类型无效")),
    };
    let mut conn = state.open_db().await.map_err(db_error)?;
    let row = sqlx::query(&format!(
        "{} SELECT * FROM enriched WHERE id=?1",
        cte(summary)
    ))
    .bind(id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(db_error)?
    .ok_or_else(|| error(StatusCode::NOT_FOUND, "记录不存在"))?;
    let chat: Option<String> = row.try_get("chat_id").map_err(db_error)?;
    let kind: Option<String> = row.try_get("chat_type").map_err(db_error)?;
    let start: Option<i64> = row.try_get("source_msg_id").map_err(db_error)?;
    let end: Option<i64> = row.try_get("source_end_msg_id").map_err(db_error)?;
    let (Some(chat), Some(kind), Some(start)) = (chat, kind, start) else {
        return Ok(Json(json!({"messages":[],"total":0,"conversation":null})));
    };
    if end.is_some_and(|e| e < start) {
        return Ok(Json(json!({"messages":[],"total":0,"conversation":null})));
    }
    let mut b=QueryBuilder::<Sqlite>::new("SELECT m.msg_id,m.sender_pid,m.nickname,m.text,m.ts,m.has_image,
        (SELECT display_name FROM persons WHERE person_id=m.sender_pid) AS name FROM messages m WHERE m.chat_id=");
    b.push_bind(chat.clone())
        .push(" AND m.chat_type=")
        .push_bind(kind.clone())
        .push(" AND m.msg_id BETWEEN ")
        .push_bind(start)
        .push(" AND ")
        .push_bind(end.unwrap_or(start));
    let owner_type: String = row.try_get("owner_type").map_err(db_error)?;
    if end.is_none() && owner_type == "person" {
        b.push(" AND m.sender_pid=")
            .push_bind(row.try_get::<String, _>("owner_id").map_err(db_error)?);
    }
    b.push(" ORDER BY m.msg_id LIMIT 61");
    let rows = b.build().fetch_all(&mut *conn).await.map_err(db_error)?;
    let messages=rows.iter().take(60).map(|r|->sqlx::Result<Value>{
        let text:Option<String>=r.try_get("text")?;
        let text=text.unwrap_or_default();
        let text=if yuantuan_core::consolidation::is_sensitive(&text) {"敏感内容已隐藏".into()} else {text.chars().take(4000).collect::<String>()};
        Ok(json!({"msg_id":r.try_get::<i64,_>("msg_id")?,"person_id":r.try_get::<String,_>("sender_pid")?,
            "name":r.try_get::<Option<String>,_>("nickname")?.filter(|s|!s.is_empty()).or(r.try_get::<Option<String>,_>("name")?),"text":text,"ts":r.try_get::<i64,_>("ts")?,"has_image":r.try_get::<bool,_>("has_image")?}))
    }).collect::<sqlx::Result<Vec<_>>>().map_err(db_error)?;
    Ok(Json(
        json!({"messages":messages,"has_more":rows.len()>60,"conversation":conversation(Some(chat),Some(kind)),"is_window":end.is_some()}),
    ))
}
