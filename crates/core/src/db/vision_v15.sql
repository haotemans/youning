CREATE INDEX IF NOT EXISTS idx_image_content_hash ON message_images(content_hash,msg_id);
CREATE TABLE IF NOT EXISTS image_corrections (
  source_msg_id INTEGER NOT NULL,
  image_index INTEGER NOT NULL,
  correction_msg_id INTEGER NOT NULL REFERENCES messages(msg_id) ON DELETE CASCADE,
  description TEXT NOT NULL,
  evidence_quote TEXT NOT NULL,
  PRIMARY KEY(source_msg_id,image_index,correction_msg_id),
  FOREIGN KEY(source_msg_id,image_index) REFERENCES message_images(msg_id,image_index) ON DELETE CASCADE
);
