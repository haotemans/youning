//! SQLite 共享入口：连接（WAL + 外键 + busy_timeout）与迁移（user_version，幂等）。
//! V0.1 表设计基线见 docs/reference/data-model.md；实际表结构以本文件的后续迁移为准。

use anyhow::{Context, Result};
mod access;
pub use access::{
    connect, database, params, params_from_iter, Connection, Database, OptionalExtension,
    SqliteExt, Statement, ToValue, Value,
};
use sqlx::Row;

/// V1 基线：data-model.md 全部 14 张表 + 索引
const V1_SQL: &str = r#"
-- 二、身份与人
CREATE TABLE IF NOT EXISTS persons (
  person_id   TEXT PRIMARY KEY,
  display_name TEXT,
  first_seen  INTEGER NOT NULL,
  last_seen   INTEGER NOT NULL,
  note        TEXT
);

CREATE TABLE IF NOT EXISTS identities (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  person_id     TEXT NOT NULL REFERENCES persons(person_id),
  platform      TEXT NOT NULL,
  platform_uid  TEXT NOT NULL,
  UNIQUE(platform, platform_uid)
);

CREATE TABLE IF NOT EXISTS member_profiles (
  chat_id    TEXT NOT NULL,
  person_id  TEXT NOT NULL REFERENCES persons(person_id),
  card       TEXT,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY(chat_id, person_id)
);

-- 三、消息流水
CREATE TABLE IF NOT EXISTS messages (
  msg_id      INTEGER PRIMARY KEY AUTOINCREMENT,
  chat_id     TEXT NOT NULL,
  chat_type   TEXT NOT NULL,
  sender_pid  TEXT NOT NULL REFERENCES persons(person_id),
  nickname    TEXT,
  text        TEXT,
  mentions    TEXT,
  reply_to    INTEGER,
  at_me       INTEGER DEFAULT 0,
  has_image   INTEGER DEFAULT 0,
  ts          INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_msg_chat_ts   ON messages(chat_id, ts);
CREATE INDEX IF NOT EXISTS idx_msg_sender_ts ON messages(sender_pid, ts);

-- 四、长期记忆与摘要
CREATE TABLE IF NOT EXISTS long_memories (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  owner_type TEXT NOT NULL,
  owner_id   TEXT NOT NULL,
  content    TEXT NOT NULL,
  source     TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_lm_owner ON long_memories(owner_type, owner_id);

CREATE TABLE IF NOT EXISTS summaries (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  owner_type   TEXT NOT NULL,
  owner_id     TEXT NOT NULL,
  period       TEXT NOT NULL,
  date         TEXT NOT NULL,
  summary      TEXT NOT NULL,
  msg_id_start INTEGER,
  msg_id_end   INTEGER,
  created_at   INTEGER NOT NULL,
  UNIQUE(owner_type, owner_id, period, date)
);

-- 五、关系系统
CREATE TABLE IF NOT EXISTS relationship_edges (
  from_pid   TEXT NOT NULL,
  to_pid     TEXT NOT NULL,
  trust      REAL DEFAULT 0.5,
  familiar   REAL DEFAULT 0.0,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY(from_pid, to_pid)
);

CREATE TABLE IF NOT EXISTS relationship_events (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  from_pid      TEXT NOT NULL,
  to_pid        TEXT NOT NULL,
  kind          TEXT NOT NULL,
  delta_familiar REAL DEFAULT 0,
  delta_trust    REAL DEFAULT 0,
  evidence      TEXT,
  created_at    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_re_pair ON relationship_events(from_pid, to_pid);

-- 六、人格版本
CREATE TABLE IF NOT EXISTS personality_versions (
  version_no INTEGER PRIMARY KEY,
  content    TEXT NOT NULL,
  note       TEXT,
  created_by TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  active     INTEGER DEFAULT 0
);

-- 七、Meme 库
CREATE TABLE IF NOT EXISTS meme_library (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  file         TEXT NOT NULL,
  category     TEXT NOT NULL,
  md5          TEXT NOT NULL,
  phash        TEXT,
  embedding    BLOB,
  added_by     TEXT NOT NULL,
  status       TEXT DEFAULT 'active',
  use_count    INTEGER DEFAULT 0,
  last_used_ts INTEGER,
  created_at   INTEGER NOT NULL
);

-- 八、Task 系统
CREATE TABLE IF NOT EXISTS tasks (
  task_id        TEXT PRIMARY KEY,
  goal           TEXT NOT NULL,
  state          TEXT NOT NULL,
  budget_max_calls INTEGER NOT NULL,
  used_calls     INTEGER DEFAULT 0,
  created_by_pid TEXT NOT NULL,
  chat_id        TEXT NOT NULL,
  created_at     INTEGER NOT NULL,
  finished_at    INTEGER
);

CREATE TABLE IF NOT EXISTS task_events (
  id      INTEGER PRIMARY KEY AUTOINCREMENT,
  task_id TEXT NOT NULL REFERENCES tasks(task_id),
  seq     INTEGER NOT NULL,
  kind    TEXT NOT NULL,
  payload TEXT,
  ts      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_te_task ON task_events(task_id, seq);

-- 九、State 持久化
CREATE TABLE IF NOT EXISTS state_kv (
  key        TEXT PRIMARY KEY,
  value      TEXT,
  updated_at INTEGER NOT NULL
);

-- 十二、事件表
CREATE TABLE IF NOT EXISTS events (
  id      INTEGER PRIMARY KEY AUTOINCREMENT,
  kind    TEXT NOT NULL,
  payload TEXT,
  ts      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_events_ts ON events(ts);
"#;

/// V2：媒体生成（Q007/Q009/Q013/Q014/Q016/Q017）—— 4 张表 + chats 增列
const V2_SQL: &str = r#"
-- 媒体 provider 表（NAI / OpenAI-Image / Gemini ...）
CREATE TABLE IF NOT EXISTS media_providers (
  name             TEXT PRIMARY KEY,
  base_url         TEXT NOT NULL,
  api_key_env      TEXT NOT NULL DEFAULT '',
  default_endpoint TEXT NOT NULL DEFAULT 'nai_native',  -- nai_native | openai_compat | gemini | xai
  enabled          INTEGER NOT NULL DEFAULT 1,
  created_at       INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL
);

-- 媒体模型表（一个 provider 下多个模型；命令 -m 入参目标）
CREATE TABLE IF NOT EXISTS media_models (
  alias            TEXT PRIMARY KEY,             -- 用户输入 ID（如 "nai4.5"）
  provider         TEXT NOT NULL REFERENCES media_providers(name),
  model_id         TEXT NOT NULL,                -- 实际请求 model id（如 "nai-diffusion-4-5-full"）
  endpoint_style   TEXT,                          -- 覆盖 provider.default_endpoint；NULL 跟 provider 默认
  prompt_style     TEXT NOT NULL DEFAULT 'natural', -- nai | anima | a1111 | natural（Q010）
  daily_quota      INTEGER NOT NULL DEFAULT 0,    -- 0 = 无上限；>0 = 每日配额
  cost_per_result  INTEGER NOT NULL DEFAULT 0,    -- 单价（0 = 免费；>0 = 计费）
  permission       TEXT NOT NULL DEFAULT 'everyone', -- admin_only | everyone（Q009）
  ratios           TEXT NOT NULL DEFAULT '',      -- 逗号分隔支持的比例（"1:1,2:3,3:2"）；空 = 用 provider 默认
  default_quality  TEXT,                          -- 默认质量档位（Seedream/Seedance 用）
  enabled          INTEGER NOT NULL DEFAULT 1,
  created_at       INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_mm_provider ON media_models(provider);

-- 媒体任务表（审计 + /tasks 命令 + 任务可视化页数据源）
CREATE TABLE IF NOT EXISTS media_tasks (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  chat_id        TEXT NOT NULL,
  chat_type      TEXT NOT NULL,
  sender_pid     TEXT NOT NULL REFERENCES persons(person_id),
  model_alias    TEXT NOT NULL,
  provider       TEXT NOT NULL,
  endpoint_style TEXT NOT NULL,
  raw_prompt     TEXT NOT NULL,
  prompt         TEXT NOT NULL,            -- 优化后的最终 prompt
  ratio          TEXT NOT NULL,
  count          INTEGER NOT NULL,
  seed           INTEGER,
  cost           INTEGER NOT NULL DEFAULT 0,
  state          TEXT NOT NULL,             -- queued | running | success | failed | canceled
  error          TEXT,
  artifacts      TEXT,                      -- JSON 数组：落盘绝对路径
  created_at     INTEGER NOT NULL,
  finished_at    INTEGER
);
CREATE INDEX IF NOT EXISTS idx_mt_chat_ts   ON media_tasks(chat_id, created_at);
CREATE INDEX IF NOT EXISTS idx_mt_sender_ts ON media_tasks(sender_pid, created_at);
CREATE INDEX IF NOT EXISTS idx_mt_state     ON media_tasks(state);

-- 用户余额表（Q009）；余额为 0 则计费模型不可用
CREATE TABLE IF NOT EXISTS media_credits (
  person_id  TEXT PRIMARY KEY REFERENCES persons(person_id),
  balance    INTEGER NOT NULL DEFAULT 0,
  updated_at INTEGER NOT NULL
);

-- Q014：chat 上次成功任务的 seed（缺省 --seed 时复用，可复现）
-- chats 表原本不存在；现以 state_kv 为通道："last_media_seed:{chat_id}" → int
-- 选 state_kv 而非新表，因为键值型写多读少，且不需 schema 变更
"#;

/// V3：LLM token 用量统计（仪表盘"今日 token"卡数据源）
const V3_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS llm_usage (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts INTEGER NOT NULL,                 -- unix 秒
    role TEXT NOT NULL,                  -- decision / bot_chat / agent_exec / optimizer / ...
    model TEXT NOT NULL,
    prompt_tokens INTEGER NOT NULL DEFAULT 0,
    completion_tokens INTEGER NOT NULL DEFAULT 0,
    total_tokens INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_llm_usage_ts ON llm_usage(ts);
CREATE INDEX IF NOT EXISTS idx_llm_usage_role_ts ON llm_usage(role, ts);
"#;

/// V4：Q55 消息流水持久化——pipeline 消费标记；重启扫 NULL 行回放进管线。
/// ALTER 和列存在性检查必须在同一个 IMMEDIATE 迁移事务内。
const V4_SQL: &str = r#"
CREATE INDEX IF NOT EXISTS idx_msg_pending ON messages(processed_at) WHERE processed_at IS NULL;
"#;

/// 迁移列表按版本升序；每步一个事务，成功后推进 user_version
const MIGRATIONS: &[(&str, &str)] = &[
    ("V0.1 基线：14 张表（data-model.md）", V1_SQL),
    (
        "V0.2 媒体生成：media_providers/models/tasks/credits 4 张表",
        V2_SQL,
    ),
    ("V0.3 LLM 用量：llm_usage 表（仪表盘 token 统计）", V3_SQL),
    (
        "V0.4 消息消费标记（Q55 恢复消费）：messages.processed_at",
        V4_SQL,
    ),
    (
        "V0.5 任务调度与列表索引",
        r#"
        CREATE INDEX IF NOT EXISTS idx_tasks_state_created ON tasks(state, created_at, task_id);
        CREATE INDEX IF NOT EXISTS idx_tasks_created ON tasks(created_at);
    "#,
    ),
    (
        "V0.6 回复来源、人物简档与记忆证据",
        r#"
        CREATE INDEX IF NOT EXISTS idx_msg_external ON messages(chat_id, chat_type, external_msg_id);
        CREATE INDEX IF NOT EXISTS idx_msg_chat_id ON messages(chat_id, msg_id);
        CREATE TABLE IF NOT EXISTS person_profile_facts (
            person_id TEXT NOT NULL REFERENCES persons(person_id),
            field TEXT NOT NULL,
            content TEXT NOT NULL,
            source_msg_id INTEGER NOT NULL,
            evidence_quote TEXT NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY(person_id, field)
        );
    "#,
    ),
    (
        "V0.7 Bot 回复归属",
        "CREATE INDEX IF NOT EXISTS idx_msg_reply_anchor ON messages(reply_anchor_id,msg_id);",
    ),
    ("V0.8 人设版本保存一问一答回复示例", "SELECT 1;"),
    (
        "V0.9 图片语境及可追溯识图记录",
        r#"
        CREATE TABLE IF NOT EXISTS message_images (
            msg_id INTEGER NOT NULL REFERENCES messages(msg_id) ON DELETE CASCADE,
            image_index INTEGER NOT NULL,
            url TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            observation TEXT,
            observed_at INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY(msg_id,image_index)
        );
    "#,
    ),
    (
        "V0.10 好友及入群申请持久审核队列",
        r#"
        CREATE TABLE IF NOT EXISTS membership_requests (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          bot_qq TEXT NOT NULL,
          request_type TEXT NOT NULL CHECK(request_type IN ('friend','group')),
          sub_type TEXT NOT NULL,
          group_id TEXT NOT NULL,
          user_id TEXT NOT NULL,
          flag TEXT NOT NULL,
          comment TEXT NOT NULL,
          status TEXT NOT NULL DEFAULT 'pending',
          note TEXT NOT NULL DEFAULT '',
          assessment TEXT,
          reviewed_at INTEGER NOT NULL DEFAULT 0,
          auto_attempts INTEGER NOT NULL DEFAULT 0,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL,
          UNIQUE(bot_qq,request_type,group_id,sub_type,flag)
        );
        CREATE INDEX IF NOT EXISTS idx_membership_pending ON membership_requests(bot_qq,status,reviewed_at,id);
        "#,
    ),
    (
        "V0.11 增量记忆批次、原文事实、会话日记及画像历史",
        r#"
        CREATE TABLE IF NOT EXISTS memory_progress (
          stage TEXT NOT NULL, owner_id TEXT NOT NULL, chat_type TEXT NOT NULL DEFAULT '',
          cursor INTEGER NOT NULL DEFAULT 0, baseline INTEGER NOT NULL DEFAULT 0, updated_at INTEGER NOT NULL DEFAULT 0,
          PRIMARY KEY(stage,owner_id,chat_type)
        );
        CREATE INDEX IF NOT EXISTS idx_msg_conversation_id ON messages(chat_id,chat_type,msg_id);
        CREATE INDEX IF NOT EXISTS idx_msg_person_id ON messages(sender_pid,msg_id);
        -- A migration baseline preserves legacy records without pretending they were
        -- processed by the new pipeline or paying to re-extract every historical chat.
        INSERT OR IGNORE INTO memory_progress(stage,owner_id,chat_type,cursor,baseline)
          SELECT 'mentions',chat_id,chat_type,MAX(msg_id),MAX(msg_id) FROM messages WHERE chat_type IN ('group','private') GROUP BY chat_id,chat_type;
        INSERT OR IGNORE INTO memory_progress(stage,owner_id,chat_type,cursor,baseline)
          SELECT 'conversation',chat_id,chat_type,MAX(msg_id),MAX(msg_id) FROM messages WHERE chat_type IN ('group','private') GROUP BY chat_id,chat_type;
        INSERT OR IGNORE INTO memory_progress(stage,owner_id,chat_type,cursor,baseline)
          SELECT 'portrait',m.sender_pid,'',MIN(MAX(m.msg_id),CAST(json_extract(CASE WHEN json_valid(s.value) THEN s.value ELSE '{}' END,'$.cutoff') AS INTEGER)),MIN(MAX(m.msg_id),CAST(json_extract(CASE WHEN json_valid(s.value) THEN s.value ELSE '{}' END,'$.cutoff') AS INTEGER))
          FROM messages m JOIN state_kv s ON s.key='profile_refresh:v1:'||m.sender_pid
          WHERE m.chat_type IN ('group','private') AND json_valid(s.value) AND CAST(json_extract(CASE WHEN json_valid(s.value) THEN s.value ELSE '{}' END,'$.cutoff') AS INTEGER)>0
          GROUP BY m.sender_pid;
        CREATE TABLE IF NOT EXISTS memory_batches (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          stage TEXT NOT NULL, owner_id TEXT NOT NULL, chat_type TEXT NOT NULL,
          from_id INTEGER NOT NULL, end_id INTEGER NOT NULL,
          status TEXT NOT NULL CHECK(status IN ('running','completed','failed')),
          started_at INTEGER NOT NULL, lease_until INTEGER NOT NULL,
          finished_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_memory_batch_running ON memory_batches(stage,owner_id,chat_type) WHERE status='running';
        CREATE INDEX IF NOT EXISTS idx_memory_batches_scope ON memory_batches(stage,owner_id,chat_type,id);
        CREATE INDEX IF NOT EXISTS idx_memory_batches_daily ON memory_batches(stage,started_at);
        CREATE TABLE IF NOT EXISTS memory_facts (
          id INTEGER PRIMARY KEY AUTOINCREMENT, batch_id INTEGER REFERENCES memory_batches(id),
          owner_type TEXT NOT NULL, owner_id TEXT NOT NULL, content TEXT NOT NULL,
          source_chat_id TEXT NOT NULL, source_chat_type TEXT NOT NULL,
          source_msg_id INTEGER NOT NULL REFERENCES messages(msg_id), evidence_quote TEXT NOT NULL,
          source_ts INTEGER NOT NULL, created_at INTEGER NOT NULL,
          UNIQUE(owner_type,owner_id,content,source_msg_id)
        );
        CREATE INDEX IF NOT EXISTS idx_memory_facts_owner ON memory_facts(owner_type,owner_id,source_msg_id);
        CREATE TABLE IF NOT EXISTS memory_diary_entries (
          id INTEGER PRIMARY KEY AUTOINCREMENT, batch_id INTEGER NOT NULL REFERENCES memory_batches(id),
          owner_type TEXT NOT NULL, owner_id TEXT NOT NULL,
          source_chat_id TEXT NOT NULL, source_chat_type TEXT NOT NULL,
          date TEXT NOT NULL, content TEXT NOT NULL,
          source_msg_id INTEGER NOT NULL, source_end_msg_id INTEGER NOT NULL,
          message_count INTEGER NOT NULL, created_at INTEGER NOT NULL,
          UNIQUE(batch_id,owner_type,owner_id)
        );
        CREATE INDEX IF NOT EXISTS idx_memory_diary_owner ON memory_diary_entries(owner_type,owner_id,date,id);
        CREATE TABLE IF NOT EXISTS person_profile_versions (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          person_id TEXT NOT NULL, field TEXT NOT NULL, content TEXT NOT NULL,
          source_msg_id INTEGER NOT NULL, evidence_quote TEXT NOT NULL,
          source_chat_id TEXT, source_chat_type TEXT, source_ts INTEGER NOT NULL,
          recorded_at INTEGER NOT NULL,
          UNIQUE(person_id,field,source_msg_id,content,evidence_quote)
        );
        CREATE INDEX IF NOT EXISTS idx_profile_versions_person ON person_profile_versions(person_id,field,source_msg_id,id);
        INSERT OR IGNORE INTO person_profile_versions(person_id,field,content,source_msg_id,evidence_quote,source_chat_id,source_chat_type,source_ts,recorded_at)
          SELECT p.person_id,p.field,p.content,p.source_msg_id,p.evidence_quote,m.chat_id,m.chat_type,p.updated_at,0
          FROM person_profile_facts p LEFT JOIN messages m ON m.msg_id=p.source_msg_id;
        CREATE TRIGGER IF NOT EXISTS profile_version_insert AFTER INSERT ON person_profile_facts BEGIN
          INSERT OR IGNORE INTO person_profile_versions(person_id,field,content,source_msg_id,evidence_quote,source_chat_id,source_chat_type,source_ts,recorded_at)
            VALUES(NEW.person_id,NEW.field,NEW.content,NEW.source_msg_id,NEW.evidence_quote,
              (SELECT chat_id FROM messages WHERE msg_id=NEW.source_msg_id),
              (SELECT chat_type FROM messages WHERE msg_id=NEW.source_msg_id),NEW.updated_at,CAST(strftime('%s','now') AS INTEGER));
        END;
        CREATE TRIGGER IF NOT EXISTS profile_version_update AFTER UPDATE ON person_profile_facts BEGIN
          INSERT OR IGNORE INTO person_profile_versions(person_id,field,content,source_msg_id,evidence_quote,source_chat_id,source_chat_type,source_ts,recorded_at)
            VALUES(NEW.person_id,NEW.field,NEW.content,NEW.source_msg_id,NEW.evidence_quote,
              (SELECT chat_id FROM messages WHERE msg_id=NEW.source_msg_id),
              (SELECT chat_type FROM messages WHERE msg_id=NEW.source_msg_id),NEW.updated_at,CAST(strftime('%s','now') AS INTEGER));
        END;
        "#,
    ),
];

pub async fn migrate(conn: &mut sqlx::SqliteConnection) -> Result<()> {
    let current: u32 = conn
        .query_row("PRAGMA user_version", crate::db::params![], |r| {
            r.try_get(0)
        })
        .await
        .context("读取 user_version 失败")?;
    if current as usize >= MIGRATIONS.len() {
        tracing::info!(user_version = current, "迁移已是最新，幂等跳过");
        return Ok(());
    }
    for (i, (desc, sql)) in MIGRATIONS.iter().enumerate() {
        let version = (i + 1) as u32;
        if current >= version {
            continue;
        }
        // 先持有写锁再检查版本和表结构，避免多个连接同时检查缺列后重复 ALTER，
        // 也避免使用启动时的旧版本覆盖另一个连接刚提交的 user_version。
        let mut tx = conn.begin_immediate().await.context("开启迁移事务失败")?;
        let locked_version: u32 = tx
            .query_row("PRAGMA user_version", crate::db::params![], |r| {
                r.try_get(0)
            })
            .await?;
        if locked_version >= version {
            continue;
        }
        if version == 4 {
            add_column_if_missing(&mut tx, "messages", "processed_at", "INTEGER").await?;
        }
        if version == 6 {
            add_column_if_missing(&mut tx, "messages", "external_msg_id", "INTEGER").await?;
            add_column_if_missing(&mut tx, "long_memories", "source_chat_id", "TEXT").await?;
            add_column_if_missing(&mut tx, "long_memories", "source_msg_id", "INTEGER").await?;
            add_column_if_missing(&mut tx, "long_memories", "source_end_msg_id", "INTEGER").await?;
        }
        if version == 7 {
            add_column_if_missing(&mut tx, "messages", "reply_anchor_id", "INTEGER").await?;
        }
        if version == 8 {
            add_column_if_missing(
                &mut tx,
                "personality_versions",
                "reply_examples",
                "TEXT NOT NULL DEFAULT '[]'",
            )
            .await?;
        }
        tx.execute_batch(sql)
            .await
            .with_context(|| format!("执行迁移 v{version} 失败"))?;
        tx.execute_batch(&format!("PRAGMA user_version = {}", version))
            .await
            .context("推进 user_version 失败")?;
        tx.commit()
            .await
            .with_context(|| format!("提交迁移 v{version} 失败"))?;
        tracing::info!(user_version = version, desc, "迁移已应用");
    }
    Ok(())
}

/// 在调用方持有迁移写事务的前提下幂等加列。
async fn add_column_if_missing(
    conn: &mut sqlx::SqliteConnection,
    table: &str,
    column: &str,
    ty: &str,
) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})")).await?;
    let cols: Vec<String> = stmt
        .query_map(crate::db::params![], |r| r.try_get::<String, _>(1))
        .await?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if cols.iter().any(|c| c == column) {
        return Ok(());
    }
    conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {ty}"))
        .await
        .with_context(|| format!("ALTER {table} ADD {column} 失败"))?;
    Ok(())
}

/// 列出用户表（排除 sqlite 内部表），供启动自检
pub async fn list_tables(conn: &mut sqlx::SqliteConnection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    ).await?;
    let names = stmt
        .query_map(crate::db::params![], |r| r.try_get(0))
        .await?
        .collect::<std::result::Result<Vec<String>, _>>()?;
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn v8_preserves_legacy_personas_and_defaults_examples_to_empty() {
        let mut conn = Connection::open_in_memory().await.unwrap();
        v3_database(&mut conn).await;
        conn.execute_batch("INSERT INTO personality_versions(version_no,content,note,created_by,created_at,active) VALUES (4,'管理员旧人设','保留','admin',123,1)").await.unwrap();
        migrate(&mut conn).await.unwrap();
        migrate(&mut conn).await.unwrap();
        let row: (i64, String, String, i64, String) = conn
            .query_row(
                "SELECT version_no,content,note,active,reply_examples FROM personality_versions",
                params![],
                |r| {
                    Ok((
                        r.try_get(0)?,
                        r.try_get(1)?,
                        r.try_get(2)?,
                        r.try_get(3)?,
                        r.try_get(4)?,
                    ))
                },
            )
            .await
            .unwrap();
        assert_eq!(
            row,
            (4, "管理员旧人设".into(), "保留".into(), 1, "[]".into())
        );
    }

    #[tokio::test]
    async fn v7_does_not_guess_old_reply_ownership_and_rolls_back_on_failure() {
        let mut conn = Connection::open_in_memory().await.unwrap();
        v3_database(&mut conn).await;
        migrate(&mut conn).await.unwrap();
        conn.execute_batch(
            "DROP INDEX idx_msg_reply_anchor; ALTER TABLE messages DROP COLUMN reply_anchor_id;
            PRAGMA user_version=6; CREATE TABLE idx_msg_reply_anchor(dummy INTEGER);",
        )
        .await
        .unwrap();
        assert!(migrate(&mut conn).await.is_err());
        assert_eq!(
            conn.query_row("PRAGMA user_version", crate::db::params![], |r| r
                .try_get::<i64, _>(0))
                .await
                .unwrap(),
            6
        );
        assert!(!conn
            .prepare("PRAGMA table_info(messages)")
            .await
            .unwrap()
            .query_map(crate::db::params![], |r| r.try_get::<String, _>(1))
            .await
            .unwrap()
            .any(|c| c.unwrap() == "reply_anchor_id"));
        conn.execute_batch("DROP TABLE idx_msg_reply_anchor")
            .await
            .unwrap();
        migrate(&mut conn).await.unwrap();
        migrate(&mut conn).await.unwrap();
        assert_eq!(
            conn.query_row(
                "SELECT reply_anchor_id FROM messages",
                crate::db::params![],
                |r| r.try_get::<Option<i64>, _>(0)
            )
            .await
            .unwrap(),
            None
        );
        assert_eq!(
            conn.query_row("SELECT text FROM messages", crate::db::params![], |r| {
                r.try_get::<String, _>(0)
            })
            .await
            .unwrap(),
            "旧消息"
        );
    }

    #[tokio::test]
    async fn v11_preserves_legacy_data_and_archives_profile_changes() {
        let mut conn = Connection::open_in_memory().await.unwrap();
        migrate(&mut conn).await.unwrap();
        conn.execute_batch("INSERT INTO persons(person_id,first_seen,last_seen) VALUES('p',1,2);
          INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts) VALUES(1,'same','group','p','我长期使用 Rust',1),(2,'same','private','p','我现在使用 Python',2);
          INSERT INTO person_profile_facts VALUES('p','technical_preferences','Rust',1,'我长期使用 Rust',1);
          INSERT INTO long_memories(owner_type,owner_id,content,source,created_at,updated_at) VALUES('chat','same','旧群记忆保留','legacy',1,1);
          INSERT INTO summaries(owner_type,owner_id,period,date,summary,created_at) VALUES('chat','same','daily','2026-11-05','旧摘要保留',1);
          INSERT INTO state_kv(key,value,updated_at) VALUES('admin_password_hash','fixture-only',1);
          DROP TRIGGER profile_version_insert; DROP TRIGGER profile_version_update;
          DROP TABLE person_profile_versions; DROP TABLE memory_diary_entries; DROP TABLE memory_facts; DROP TABLE memory_batches; DROP TABLE memory_progress;
          DROP INDEX idx_msg_conversation_id; DROP INDEX idx_msg_person_id; PRAGMA user_version=10;").await.unwrap();
        migrate(&mut conn).await.unwrap();
        migrate(&mut conn).await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages")
                .fetch_one(&mut *conn)
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT content FROM long_memories")
                .fetch_one(&mut *conn)
                .await
                .unwrap(),
            "旧群记忆保留"
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT summary FROM summaries")
                .fetch_one(&mut *conn)
                .await
                .unwrap(),
            "旧摘要保留"
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>(
                "SELECT value FROM state_kv WHERE key='admin_password_hash'"
            )
            .fetch_one(&mut *conn)
            .await
            .unwrap(),
            "fixture-only"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM person_profile_versions")
                .fetch_one(&mut *conn)
                .await
                .unwrap(),
            1
        );
        conn.execute_batch("UPDATE person_profile_facts SET content='Python',source_msg_id=2,evidence_quote='我现在使用 Python',updated_at=2").await.unwrap();
        assert_eq!(
            crate::memory::profile(&mut conn, "p", 1).await.unwrap()[0].content,
            "Rust"
        );
        assert_eq!(
            crate::memory::profile(&mut conn, "p", 2).await.unwrap()[0].content,
            "Python"
        );
        let kinds: Vec<String> = sqlx::query_scalar(
            "SELECT source_chat_type FROM person_profile_versions ORDER BY source_msg_id",
        )
        .fetch_all(&mut *conn)
        .await
        .unwrap();
        assert_eq!(kinds, vec!["group", "private"]);
    }

    async fn v3_database(conn: &mut sqlx::SqliteConnection) {
        conn.execute_batch(V1_SQL).await.unwrap();
        conn.execute_batch(V2_SQL).await.unwrap();
        conn.execute_batch(V3_SQL).await.unwrap();
        conn.execute_batch(&format!("PRAGMA user_version = {}", 3))
            .await
            .unwrap();
        conn.execute_batch("INSERT INTO persons VALUES ('p_test', 'test', 1, 1, NULL);
            INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts) VALUES ('g_test','group','p_test','旧消息',1);
            INSERT INTO tasks(task_id,goal,state,budget_max_calls,created_by_pid,chat_id,created_at)
            VALUES ('t_test','旧任务','running',10,'p_test','g_test',1);").await.unwrap();
    }

    #[tokio::test]
    async fn migration_preserves_v3_rows_and_indexes_scheduler_and_lists() {
        let mut conn = Connection::open_in_memory().await.unwrap();
        v3_database(&mut conn).await;
        migrate(&mut conn).await.unwrap();
        migrate(&mut conn).await.unwrap();
        let row: (String, Option<i64>) = conn
            .query_row(
                "SELECT text, processed_at FROM messages",
                crate::db::params![],
                |r| Ok((r.try_get(0)?, r.try_get(1)?)),
            )
            .await
            .unwrap();
        assert_eq!(row, ("旧消息".into(), None));
        assert_eq!(
            conn.query_row("SELECT goal FROM tasks", crate::db::params![], |r| r
                .try_get::<String, _>(0))
                .await
                .unwrap(),
            "旧任务"
        );
        for (query, index) in [
            ("SELECT task_id, chat_id, goal, used_calls FROM tasks WHERE state='running' ORDER BY created_at,task_id LIMIT 6", "idx_tasks_state_created"),
            ("SELECT * FROM tasks ORDER BY created_at DESC LIMIT 50", "idx_tasks_created"),
        ] {
            let plan = conn.prepare(&format!("EXPLAIN QUERY PLAN {query}")).await.unwrap()
                .query_map( crate::db::params![], |r| r.try_get::<String, _>(3)).await.unwrap()
                .collect::<sqlx::Result<Vec<_>>>().unwrap().join("\n");
            assert!(plan.contains(index), "{plan}");
            assert!(!plan.contains("TEMP B-TREE"), "{plan}");
        }
    }

    #[tokio::test]
    async fn concurrent_migrations_serialize_schema_changes() {
        let path = std::env::temp_dir().join(format!(
            "yuantuan-migrate-{}-{}.db",
            std::process::id(),
            rand::random::<u64>()
        ));
        let mut conn = connect(&path).await.unwrap();
        let database = conn.database();
        v3_database(&mut conn).await;
        drop(conn);
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(4));
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let path = path.clone();
                let barrier = barrier.clone();
                tokio::spawn(async move {
                    let mut conn = connect(&path).await.unwrap();
                    barrier.wait().await;
                    migrate(&mut conn).await.unwrap();
                })
            })
            .collect();
        for thread in threads {
            thread.await.unwrap();
        }
        let mut conn = connect(&path).await.unwrap();
        migrate(&mut conn).await.unwrap();
        assert!(std::sync::Arc::ptr_eq(&database, &conn.database()));
        assert_eq!(
            conn.query_row("PRAGMA user_version", crate::db::params![], |r| r
                .try_get::<i64, _>(0))
                .await
                .unwrap(),
            MIGRATIONS.len() as i64
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM messages", crate::db::params![], |r| r
                .try_get::<i64, _>(0))
                .await
                .unwrap(),
            1
        );
        drop(conn);
        database.close().await;
        assert_eq!(database.pool().size(), 0);
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn v6_preserves_legacy_memories_without_fabricating_sources_and_rolls_back_failure() {
        let mut conn = Connection::open_in_memory().await.unwrap();
        v3_database(&mut conn).await;
        add_column_if_missing(&mut conn, "messages", "processed_at", "INTEGER")
            .await
            .unwrap();
        conn.execute_batch(V4_SQL).await.unwrap();
        conn.execute_batch(MIGRATIONS[4].1).await.unwrap();
        conn.execute_batch(&format!("PRAGMA user_version = {}", 5))
            .await
            .unwrap();
        conn.execute_batch("INSERT INTO long_memories(owner_type,owner_id,content,source,created_at,updated_at) VALUES ('person','p_test','旧记忆','explicit',1,1); CREATE TABLE idx_msg_external(dummy INTEGER);").await.unwrap();
        assert!(migrate(&mut conn).await.is_err());
        assert_eq!(
            conn.query_row("PRAGMA user_version", crate::db::params![], |r| r
                .try_get::<i64, _>(0))
                .await
                .unwrap(),
            5
        );
        assert!(conn
            .prepare("SELECT external_msg_id FROM messages")
            .await
            .is_err());
        conn.execute_batch("DROP TABLE idx_msg_external")
            .await
            .unwrap();
        migrate(&mut conn).await.unwrap();
        migrate(&mut conn).await.unwrap();
        let row: (String, Option<i64>, Option<String>) = conn
            .query_row(
                "SELECT content,source_msg_id,source_chat_id FROM long_memories",
                crate::db::params![],
                |r| Ok((r.try_get(0)?, r.try_get(1)?, r.try_get(2)?)),
            )
            .await
            .unwrap();
        assert_eq!(row, ("旧记忆".into(), None, None));
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM person_profile_facts",
                crate::db::params![],
                |r| r.try_get::<i64, _>(0)
            )
            .await
            .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn failed_v4_migration_rolls_back_added_column_and_version() {
        let mut conn = Connection::open_in_memory().await.unwrap();
        v3_database(&mut conn).await;
        // Index/table name collision fails after V4's ALTER TABLE.
        conn.execute_batch("CREATE TABLE idx_msg_pending(dummy INTEGER)")
            .await
            .unwrap();
        assert!(migrate(&mut conn).await.is_err());
        let columns = conn
            .prepare("PRAGMA table_info(messages)")
            .await
            .unwrap()
            .query_map(crate::db::params![], |r| r.try_get::<String, _>(1))
            .await
            .unwrap()
            .collect::<sqlx::Result<Vec<_>>>()
            .unwrap();
        assert!(!columns.iter().any(|name| name == "processed_at"));
        assert_eq!(
            conn.query_row("PRAGMA user_version", crate::db::params![], |r| r
                .try_get::<i64, _>(0))
                .await
                .unwrap(),
            3
        );
        conn.execute_batch("DROP TABLE idx_msg_pending")
            .await
            .unwrap();
        migrate(&mut conn).await.unwrap();
    }
}
