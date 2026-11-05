//! User-provided Cloud sticker sheets, extracted deterministically on first start.
use crate::db::{params, SqliteExt};
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use sqlx::Row;
use std::{
    io::{Cursor, Write},
    path::{Path, PathBuf},
};

const STAMP: &str = "meme_pack:cloud:v1";
const SHEETS: [&[u8]; 4] = [
    include_bytes!("../assets/cloud-memes/01.jpg"),
    include_bytes!("../assets/cloud-memes/02.jpg"),
    include_bytes!("../assets/cloud-memes/03.jpg"),
    include_bytes!("../assets/cloud-memes/04.jpg"),
];

#[derive(Deserialize)]
struct Manifest {
    grid: u32,
    margin: u32,
    sheets: Vec<Sheet>,
}
#[derive(Deserialize)]
struct Sheet {
    cells: Vec<Cell>,
}
#[derive(Deserialize)]
struct Cell {
    category: String,
    #[serde(default = "default_enabled")]
    enabled: bool,
}
fn default_enabled() -> bool {
    true
}
struct Sticker {
    relative: PathBuf,
    category: String,
    bytes: Vec<u8>,
}

fn extract() -> Result<Vec<Sticker>> {
    let manifest: Manifest =
        serde_json::from_str(include_str!("../assets/cloud-memes/manifest.json"))?;
    ensure!(
        manifest.grid == 4 && manifest.sheets.len() == 4,
        "云团表情包布局不符"
    );
    let mut stickers = Vec::new();
    for (sheet_index, (sheet, bytes)) in manifest.sheets.iter().zip(SHEETS).enumerate() {
        ensure!(sheet.cells.len() == 16, "每张拼图应为 16 格");
        let image = image::load_from_memory(bytes)?;
        ensure!(
            image.width() == 2048 && image.height() == 2048,
            "拼图尺寸不符"
        );
        let side = image.width() / manifest.grid;
        let margin = manifest.margin;
        ensure!(margin * 2 < side, "裁切边距无效");
        for (index, cell) in sheet.cells.iter().enumerate() {
            if !cell.enabled {
                continue;
            }
            ensure!(
                !cell.category.is_empty()
                    && Path::new(&cell.category).components().count() == 1
                    && !cell.category.chars().any(|c| "\\/:*?\"<>|".contains(c)),
                "表情分类无效"
            );
            let cropped = image.crop_imm(
                (index as u32 % 4) * side + margin,
                (index as u32 / 4) * side + margin,
                side - 2 * margin,
                side - 2 * margin,
            );
            let mut output = Cursor::new(Vec::new());
            cropped.write_to(&mut output, image::ImageFormat::Png)?;
            stickers.push(Sticker {
                relative: Path::new(&cell.category).join(format!(
                    "cloud-v1-{:02}-{:02}.png",
                    sheet_index + 1,
                    index + 1
                )),
                category: cell.category.clone(),
                bytes: output.into_inner(),
            });
        }
    }
    Ok(stickers)
}

/// Install once before the ordinary scanner. Curated text variants use exact
/// hashes only: perceptual similarity must not discard different captions.
/// The completion stamp preserves later admin rejection/deletion and recategorisation.
pub async fn install(db: &Path, root: &Path) -> Result<usize> {
    let mut conn = crate::db::connect(db).await?;
    let installed: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM state_kv WHERE key=?1)",
            params![STAMP],
            |r| r.try_get(0),
        )
        .await?;
    if installed {
        return Ok(0);
    }
    let stickers = tokio::task::spawn_blocking(extract)
        .await
        .context("拆分表情包任务失败")??;
    std::fs::create_dir_all(root)?;
    let canonical_root = root.canonicalize()?;
    let mut tx = conn.begin_immediate().await?;
    let installed: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM state_kv WHERE key=?1)",
            params![STAMP],
            |r| r.try_get(0),
        )
        .await?;
    if installed {
        return Ok(0);
    }
    let mut added = 0;
    for sticker in stickers {
        let file = sticker.relative.to_string_lossy().replace('\\', "/");
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM meme_library WHERE file=?1)",
                params![file],
                |r| r.try_get(0),
            )
            .await?;
        if exists {
            continue;
        }
        let path = canonical_root.join(&sticker.relative);
        let parent = path.parent().context("表情路径缺少目录")?;
        std::fs::create_dir_all(parent)?;
        ensure!(
            parent.canonicalize()?.starts_with(&canonical_root),
            "表情路径越出图库"
        );
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut out) => out.write_all(&sticker.bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
        ensure!(
            path.canonicalize()?.starts_with(&canonical_root) && path.is_file(),
            "表情路径无效"
        );
        let actual = std::fs::read(&path)?;
        let md5 = crate::meme::compute_md5(&actual);
        let phash = crate::meme::compute_dhash(&actual).map(|h| format!("{h:016x}"));
        let count = tx.execute("INSERT INTO meme_library(file,category,md5,phash,added_by,status,use_count,created_at)
            SELECT ?1,?2,?3,?4,'admin','active',0,unixepoch() WHERE NOT EXISTS(SELECT 1 FROM meme_library WHERE md5=?3)",
            params![file,sticker.category,md5,phash]).await?;
        added += count as usize;
    }
    tx.execute(
        "INSERT INTO state_kv(key,value,updated_at) VALUES(?1,'installed',unixepoch())",
        params![STAMP],
    )
    .await?;
    tx.commit().await?;
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn all_60_unique_captions_install_once_and_admin_changes_survive() {
        let dir = std::env::temp_dir().join(format!(
            "yt-cloud-pack-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("db.sqlite");
        let root = dir.join("memes");
        let mut conn = crate::db::connect(&db).await.unwrap();
        crate::db::migrate(&mut conn).await.unwrap();
        assert_eq!(install(&db, &root).await.unwrap(), 60);
        assert_eq!(install(&db, &root).await.unwrap(), 0);
        let (count, categories): (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*),COUNT(DISTINCT category) FROM meme_library WHERE status='active'",
                params![],
                |r| Ok((r.try_get(0)?, r.try_get(1)?)),
            )
            .await
            .unwrap();
        assert_eq!((count, categories), (60, 60));
        conn.execute_batch("INSERT INTO persons(person_id,first_seen,last_seen) VALUES('p_pack',1,1);
            INSERT INTO messages(msg_id,chat_id,chat_type,sender_pid,text,ts) VALUES(1,'g','group','p_pack','发表情',1);").await.unwrap();
        let msg = crate::event::MessageReceivedPayload {
            msg_id: 1,
            chat_id: "g".into(),
            chat_type: "group".into(),
            sender_pid: "p_pack".into(),
            text: "发表情".into(),
            at_me: true,
            has_image: false,
            reply_to: None,
            sender_bot: false,
            image_urls: vec![],
            ts: 1,
        };
        let snapshot = crate::context_builder::capture_reply_snapshot(
            &db,
            &msg,
            1,
            &crate::context_builder::ContextCfg::default(),
        )
        .await
        .unwrap();
        assert_eq!(snapshot.meme_categories.len(), 60, "不能只暴露最初的24类");
        assert!(snapshot.meme_categories.contains(&"晚安".to_owned()));
        assert!(snapshot.meme_categories.contains(&"麦霸".to_owned()));
        let removed: String = conn
            .query_row("SELECT file FROM meme_library WHERE id=1", params![], |r| {
                r.try_get(0)
            })
            .await
            .unwrap();
        let image = image::open(root.join(&removed)).unwrap();
        assert_eq!((image.width(), image.height()), (504, 504));
        std::fs::remove_file(root.join(&removed)).unwrap();
        conn.execute_batch("DELETE FROM meme_library WHERE id=1; UPDATE meme_library SET category='我的分类',status='pending' WHERE id=2;").await.unwrap();
        assert_eq!(install(&db, &root).await.unwrap(), 0);
        assert!(!root.join(removed).exists());
        crate::meme::scan_and_ingest(&root, &db).await.unwrap();
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM meme_library", params![], |r| {
                r.try_get(0)
            })
            .await
            .unwrap();
        assert_eq!(remaining, 59);
        let status: String = conn
            .query_row(
                "SELECT status FROM meme_library WHERE id=2 AND category='我的分类'",
                params![],
                |r| r.try_get(0),
            )
            .await
            .unwrap();
        assert_eq!(status, "pending");
    }
}
