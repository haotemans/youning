//! Exact public sticker provenance is distinct from recognizing a real person.
use super::*;

#[derive(Debug, Serialize)]
pub(super) struct Asset {
    pub id: i64,
    pub category: String,
}
impl Asset {
    pub fn observation(&self) -> Observation {
        Observation {
            summary: format!(
                "图片原文件与已登记的云团专属「{}」表情完全一致，是云团的虚构角色表情。",
                self.category
            ),
            visible_text: String::new(),
            uncertainty: "确认的是图库登记的虚构角色和类别，未推断未描述的画面细节或现实人物身份。"
                .into(),
        }
    }
    pub fn context(&self, sender: &str) -> Value {
        json!({"meme_id":self.id,"category":self.category,"pack":"self","character_name":"云团",
            "owner_person_id":"self","source":if sender=="self" {"sent_meme"} else {"exact_file_match"},
            "real_person_identity":false})
    }
}

pub(super) async fn read_file(root: &Path, file: &str) -> Result<Vec<u8>> {
    let relative = Path::new(file);
    ensure!(
        relative
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_))),
        "图片路径无效"
    );
    let root = tokio::fs::canonicalize(root).await?;
    let path = tokio::fs::canonicalize(root.join(relative)).await?;
    ensure!(path.starts_with(&root) && path.is_file(), "图片路径无效");
    use tokio::io::AsyncReadExt;
    let mut bytes = Vec::new();
    tokio::fs::File::open(path)
        .await?
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .await?;
    ensure!(
        !bytes.is_empty() && bytes.len() <= 8 * 1024 * 1024,
        "图片为空或超过8MB"
    );
    Ok(bytes)
}

fn asset(row: &sqlx::sqlite::SqliteRow) -> Option<Asset> {
    let category: String = row.try_get("category").ok()?;
    if category.trim().is_empty()
        || category.chars().count() > 40
        || crate::consolidation::is_sensitive(&category)
    {
        return None;
    }
    Some(Asset {
        id: row.try_get("id").ok()?,
        category,
    })
}

pub(super) async fn metadata(
    c: &mut sqlx::SqliteConnection,
    id: Option<i64>,
) -> Result<Option<Asset>> {
    let Some(id) = id else { return Ok(None) };
    let row=sqlx::query(&format!("SELECT id,category FROM meme_library WHERE id=?1 AND status='active' AND added_by='admin' AND {}",crate::meme::SELF_FILE_SQL))
        .bind(id).fetch_optional(&mut *c).await?;
    Ok(row.as_ref().and_then(asset))
}

pub(super) async fn find(
    c: &mut sqlx::SqliteConnection,
    root: &Path,
    bytes: &[u8],
) -> Result<Option<Asset>> {
    let hash = crate::meme::compute_md5(bytes);
    let rows=sqlx::query(&format!("SELECT id,file,category FROM meme_library WHERE md5=?1 AND status='active' AND added_by='admin' AND {} ORDER BY id LIMIT 4",crate::meme::SELF_FILE_SQL))
        .bind(hash).fetch_all(&mut *c).await?;
    for row in rows {
        let Some(asset) = asset(&row) else { continue };
        let file: String = row.try_get("file")?;
        // MD5 only narrows candidates. The actual file bytes must match, too.
        if read_file(root, &file)
            .await
            .is_ok_and(|registered| registered == bytes)
        {
            return Ok(Some(asset));
        }
    }
    Ok(None)
}
