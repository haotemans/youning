-- processed_at remains the compatibility completion flag. No source text is removed.
UPDATE messages SET decision_state='done' WHERE processed_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_msg_decision_pending
  ON messages(decision_state, decision_retry_at, chat_type, chat_id, msg_id)
  WHERE processed_at IS NULL AND sender_pid != 'self' AND sender_bot=0;
