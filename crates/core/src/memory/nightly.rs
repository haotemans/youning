//! A bounded nightly queue. Attempts, per-person markers and spacing survive restarts.
use crate::{
    db::{self, params, SqliteExt},
    llm::{LlmGateway, Role},
};
use anyhow::Result;
use std::{
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

static RUNNING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub struct Options {
    pub date: String,
    pub deadline: i64,
    pub limit: usize,
    pub interval_secs: u64,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

enum Claim {
    Person(String),
    Wait(u64),
    Done,
}

async fn claim(path: &Path, options: &Options, at: i64) -> Result<Claim> {
    if at >= options.deadline || options.limit == 0 {
        return Ok(Claim::Done);
    }
    let mut conn = db::connect(path).await?;
    let mut tx = conn.transaction().await?;
    let key = format!("profile_nightly:v1:{}", options.date);
    tx.execute("INSERT INTO state_kv(key,value,updated_at) VALUES(?1,'{\"used\":0,\"next_at\":0}',?2) ON CONFLICT(key) DO NOTHING",params![key,at]).await?;
    let stored: String = sqlx::query_scalar("SELECT value FROM state_kv WHERE key=?1")
        .bind(&key)
        .fetch_one(&mut *tx)
        .await?;
    let state: serde_json::Value = serde_json::from_str(&stored)?;
    let used = state["used"].as_u64().unwrap_or(0);
    if used >= options.limit.min(100) as u64 {
        return Ok(Claim::Done);
    }
    let next = state["next_at"].as_i64().unwrap_or(0);
    if next > at {
        return Ok(Claim::Wait((next - at) as u64));
    }
    let prefix = format!("profile_nightly_person:v1:{}:", options.date);
    let person=sqlx::query_scalar::<_,String>(
        "SELECT m.sender_pid FROM messages m
         LEFT JOIN state_kv s ON s.key='profile_refresh:v1:'||m.sender_pid
         LEFT JOIN memory_progress progress ON progress.stage='portrait' AND progress.owner_id=m.sender_pid AND progress.chat_type=''
         WHERE m.sender_pid<>'self' AND m.chat_type IN ('group','private')
           AND length(trim(COALESCE(m.text,'')))>=4
           AND NOT EXISTS(SELECT 1 FROM state_kv done WHERE done.key=?1||m.sender_pid)
         GROUP BY m.sender_pid
         HAVING MAX(m.msg_id)>COALESCE(progress.cursor,0)
           AND COALESCE(json_extract(CASE WHEN json_valid(s.value) THEN s.value ELSE '{}' END,'$.attempt'),0)<=?2-300
         ORDER BY CASE WHEN COALESCE(s.updated_at,0)<?2-604800 THEN 0 ELSE 1 END,
         CASE WHEN EXISTS(SELECT 1 FROM memory_facts f WHERE f.owner_type='person' AND f.owner_id=m.sender_pid AND f.source_msg_id>COALESCE(progress.cursor,0)) THEN 0 ELSE 1 END,
         COALESCE(s.updated_at,0),MAX(m.ts) DESC,m.sender_pid LIMIT 1")
        .bind(prefix.clone()).bind(at).fetch_optional(&mut *tx).await?;
    let Some(person) = person else {
        return Ok(Claim::Done);
    };
    // Compare-and-swap the shared daily row before committing the person's attempt.
    // Unknown/cancelled work holds the queue for five minutes plus the configured gap.
    let claimed=tx.execute("UPDATE state_kv SET value=?3,updated_at=?4 WHERE key=?1 AND value=?2",
        params![key,stored,serde_json::json!({"used":used+1,"next_at":at+300+options.interval_secs.min(3600) as i64,"active_person":person}).to_string(),at]).await?;
    if claimed == 0 {
        return Ok(Claim::Wait(1));
    }
    tx.execute(
        "INSERT INTO state_kv(key,value,updated_at) VALUES(?1,'started',?2)",
        params![format!("{prefix}{person}"), at],
    )
    .await?;
    tx.commit().await?;
    Ok(Claim::Person(person))
}

pub async fn run(path: &Path, gw: &LlmGateway, options: &Options) -> Result<()> {
    if gw.role(Role::Decision).is_none() {
        return Ok(());
    }
    let _guard = RUNNING.lock().await;
    loop {
        match claim(path, options, now()).await? {
            Claim::Done => return Ok(()),
            Claim::Wait(seconds) => {
                let remaining = (options.deadline - now()).max(0) as u64;
                if remaining == 0 {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_secs(seconds.min(remaining))).await;
            }
            Claim::Person(person) => {
                let result = tokio::time::timeout(
                    Duration::from_secs(180),
                    super::refresh::person(path, gw, &person, false),
                )
                .await;
                if !matches!(result, Ok(Ok(_))) {
                    tracing::warn!(person_id=%person,"夜间画像整理失败或超时，保留现有资料");
                }
                let at = now();
                let mut conn = db::connect(path).await?;
                conn.execute("UPDATE state_kv SET value=json_set(value,'$.next_at',?3,'$.active_person',NULL),updated_at=?4 WHERE key=?1 AND json_extract(value,'$.active_person')=?2",
                    params![format!("profile_nightly:v1:{}",options.date),person,at+options.interval_secs.min(3600) as i64,at]).await?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn claim_persists_spacing_quota_and_person_deduplication() {
        let path = std::env::temp_dir().join(format!("nightly-claim-{}.db", rand::random::<u64>()));
        let mut conn = db::connect(&path).await.unwrap();
        db::migrate(&mut conn).await.unwrap();
        conn.execute_batch("INSERT INTO persons(person_id,first_seen,last_seen) VALUES('a',1,1),('b',1,1);
          INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts) VALUES('g','group','a','我长期使用 Rust',1),('p','private','b','我长期使用 Python',2);").await.unwrap();
        let options = Options {
            date: "2026-11-06".into(),
            deadline: 5000,
            limit: 1,
            interval_secs: 60,
        };
        let Claim::Person(first) = claim(&path, &options, 1000).await.unwrap() else {
            panic!("missing first person")
        };
        assert_eq!(first, "b");
        // Raising the cap cannot defeat persisted spacing or select b a second time.
        let more = Options {
            limit: 2,
            ..options
        };
        assert!(matches!(
            claim(&path, &more, 1100).await.unwrap(),
            Claim::Wait(260)
        ));
        assert!(matches!(claim(&path,&more,1360).await.unwrap(),Claim::Person(p) if p=="a"));
        assert!(matches!(
            claim(&path, &more, 2000).await.unwrap(),
            Claim::Done
        ));
        assert!(matches!(
            claim(&path, &more, 5000).await.unwrap(),
            Claim::Done
        ));
    }
}
