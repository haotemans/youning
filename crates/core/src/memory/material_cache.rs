//! Bounded read-through material cache. SQLite epochs invalidate edits/deletes;
//! scope, database and snapshot version are part of every key.
use anyhow::Result;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
type Entries = BTreeMap<String, (Instant, Vec<Value>)>;
fn entries() -> &'static Mutex<Entries> {
    static CACHE: OnceLock<Mutex<Entries>> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
}
pub async fn key(
    c: &mut sqlx::SqliteConnection,
    kind: &str,
    owner: &str,
    chat: &str,
    chat_type: &str,
    cutoff: i64,
) -> Result<Option<String>> {
    let file: String =
        sqlx::query_scalar("SELECT file FROM pragma_database_list WHERE name='main'")
            .fetch_one(&mut *c)
            .await?;
    if file.is_empty() {
        return Ok(None);
    }
    let epoch: i64 = sqlx::query_scalar("SELECT epoch FROM memory_material_epoch WHERE id=1")
        .fetch_one(&mut *c)
        .await?;
    let version: i64 = if kind == "legacy" {
        sqlx::query_scalar("SELECT COALESCE(MAX(id),0) FROM person_profile_versions WHERE person_id=? AND source_msg_id<=?").bind(owner).bind(cutoff).fetch_one(&mut *c).await?
    } else {
        sqlx::query_scalar("SELECT COALESCE(MAX(id),0) FROM memory_items WHERE owner_type=? AND owner_id=? AND source_end_id<=?").bind(kind).bind(owner).bind(cutoff).fetch_one(&mut *c).await?
    };
    Ok(Some(
        serde_json::json!([file, kind, owner, chat, chat_type, epoch, version]).to_string(),
    ))
}
pub fn get(key: &Option<String>) -> Option<Vec<Value>> {
    let cache = entries().lock().unwrap();
    let (time, value) = cache.get(key.as_ref()?)?;
    (time.elapsed() < Duration::from_secs(60)).then(|| value.clone())
}
pub fn put(key: Option<String>, value: &[Value]) {
    let Some(key) = key else { return };
    let mut cache = entries().lock().unwrap();
    cache.retain(|_, (time, _)| time.elapsed() < Duration::from_secs(60));
    if cache.len() >= 128 {
        if let Some(oldest) = cache
            .iter()
            .min_by_key(|(_, v)| v.0)
            .map(|(k, _)| k.clone())
        {
            cache.remove(&oldest);
        }
    }
    cache.insert(key, (Instant::now(), value.to_vec()));
}
