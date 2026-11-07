-- Additive migration: old messages, profiles, diaries and task events remain intact.
CREATE TABLE IF NOT EXISTS memory_items (
 id INTEGER PRIMARY KEY, owner_type TEXT NOT NULL, owner_id TEXT NOT NULL,
 item_key TEXT NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('fact','inference','event','portrait')),
 content TEXT NOT NULL, status TEXT NOT NULL CHECK(status IN ('current','ended','uncertain','corrected')),
 source_chat_id TEXT NOT NULL, source_chat_type TEXT NOT NULL,
 source_end_id INTEGER NOT NULL, evidence TEXT NOT NULL CHECK(json_valid(evidence)),
 valid_until INTEGER, recorded_at INTEGER NOT NULL,
 UNIQUE(owner_type,owner_id,item_key,source_chat_type,source_chat_id,source_end_id,content,status)
);
CREATE INDEX IF NOT EXISTS memory_items_owner ON memory_items(owner_type,owner_id,source_end_id);
CREATE INDEX IF NOT EXISTS messages_scope_time ON messages(chat_type,chat_id,ts,msg_id);
CREATE TABLE IF NOT EXISTS task_runtime (
 task_id TEXT PRIMARY KEY REFERENCES tasks(task_id), root_id TEXT NOT NULL, parent_id TEXT,
 chat_type TEXT NOT NULL, anchor_id INTEGER, revision INTEGER NOT NULL DEFAULT 1,
 phase TEXT NOT NULL DEFAULT 'queued', history TEXT NOT NULL DEFAULT '[]',
 allowed_tools TEXT NOT NULL DEFAULT '[]', checkpoint TEXT NOT NULL DEFAULT '{}',
 lease_until INTEGER NOT NULL DEFAULT 0, lease_owner TEXT,
 deadline INTEGER NOT NULL, max_tokens INTEGER NOT NULL, spent_tokens INTEGER NOT NULL DEFAULT 0,
 no_progress INTEGER NOT NULL DEFAULT 0, stop_reason TEXT,
 delivery_state TEXT NOT NULL DEFAULT 'none', final_text TEXT,
 updated_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS task_origin_once ON task_runtime(chat_type,anchor_id) WHERE parent_id IS NULL AND anchor_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS task_runtime_queue ON task_runtime(phase,updated_at);
CREATE TABLE IF NOT EXISTS task_invocations (
 id TEXT PRIMARY KEY, task_id TEXT NOT NULL REFERENCES tasks(task_id), revision INTEGER NOT NULL,
 tool_name TEXT NOT NULL, args TEXT NOT NULL, state TEXT NOT NULL,
 result TEXT, started_at INTEGER NOT NULL, finished_at INTEGER
);
CREATE INDEX IF NOT EXISTS task_invocation_task ON task_invocations(task_id,revision,started_at);
CREATE TABLE IF NOT EXISTS task_outbox (
 task_id TEXT NOT NULL REFERENCES tasks(task_id), revision INTEGER NOT NULL,
 text TEXT NOT NULL, artifacts TEXT NOT NULL DEFAULT '[]', state TEXT NOT NULL DEFAULT 'ready', receipt TEXT,
 updated_at INTEGER NOT NULL, PRIMARY KEY(task_id,revision)
);
CREATE TABLE IF NOT EXISTS capability_catalog (
 name TEXT NOT NULL, kind TEXT NOT NULL, description TEXT NOT NULL, schema_json TEXT NOT NULL,
 enabled INTEGER NOT NULL DEFAULT 1, scopes TEXT NOT NULL DEFAULT '[]',
 updated_at INTEGER NOT NULL, PRIMARY KEY(name,kind)
);
CREATE TABLE IF NOT EXISTS capability_invocations (
 id INTEGER PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL,
 chat_id TEXT NOT NULL, chat_type TEXT NOT NULL, actor TEXT NOT NULL,
 anchor_id INTEGER, state TEXT NOT NULL, started_at INTEGER NOT NULL, finished_at INTEGER,
 summary TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS capability_origin_once ON capability_invocations(name,kind,chat_type,chat_id,anchor_id) WHERE anchor_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS memory_material_epoch(id INTEGER PRIMARY KEY CHECK(id=1),epoch INTEGER NOT NULL);
INSERT OR IGNORE INTO memory_material_epoch VALUES(1,0);
CREATE TRIGGER IF NOT EXISTS cache_memory_items_insert AFTER INSERT ON memory_items BEGIN UPDATE memory_material_epoch SET epoch=epoch+1 WHERE id=1; END;
CREATE TRIGGER IF NOT EXISTS cache_memory_items_update AFTER UPDATE ON memory_items BEGIN UPDATE memory_material_epoch SET epoch=epoch+1 WHERE id=1; END;
CREATE TRIGGER IF NOT EXISTS cache_memory_items_delete AFTER DELETE ON memory_items BEGIN UPDATE memory_material_epoch SET epoch=epoch+1 WHERE id=1; END;
CREATE TRIGGER IF NOT EXISTS cache_person_profile_versions_insert AFTER INSERT ON person_profile_versions BEGIN UPDATE memory_material_epoch SET epoch=epoch+1 WHERE id=1; END;
CREATE TRIGGER IF NOT EXISTS cache_person_profile_versions_update AFTER UPDATE ON person_profile_versions BEGIN UPDATE memory_material_epoch SET epoch=epoch+1 WHERE id=1; END;
CREATE TRIGGER IF NOT EXISTS cache_person_profile_versions_delete AFTER DELETE ON person_profile_versions BEGIN UPDATE memory_material_epoch SET epoch=epoch+1 WHERE id=1; END;
CREATE TRIGGER IF NOT EXISTS cache_messages_edit AFTER UPDATE OF text,sender_pid,chat_id,chat_type,ts ON messages BEGIN UPDATE memory_material_epoch SET epoch=epoch+1 WHERE id=1; END;
CREATE TRIGGER IF NOT EXISTS cache_messages_delete AFTER DELETE ON messages BEGIN UPDATE memory_material_epoch SET epoch=epoch+1 WHERE id=1; END;
