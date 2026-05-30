use std::path::Path;

use anyhow::{Result, bail};

use crate::mdstore::{db, files, ingest, status};

pub fn sync_dir(dir: &Path, db_path: &Path) -> Result<status::StatusSummary> {
    if !dir.is_dir() {
        bail!("sync path is not a directory: {}", dir.display());
    }

    db::init(db_path)?;
    let diff = status::diff_dir(dir, db_path)?;
    let summary = diff.summary();
    let synced_at = files::now_epoch_ns()?;
    let conn = db::open(db_path)?;

    for change in diff.new.iter().chain(diff.modified.iter()) {
        let document = ingest::document_from_file(dir, &change.path, synced_at)?;
        db::upsert_document(&conn, &document)?;
    }

    for path in &diff.deleted {
        db::soft_delete_document(&conn, path, synced_at)?;
    }

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use rusqlite::OptionalExtension;

    use crate::mdstore::db::DocumentUpsert;

    use super::*;

    #[test]
    fn sync_applies_new_modified_deleted_and_unchanged_documents() -> Result<()> {
        let root = unique_temp_dir("mdlite-sync");
        fs::create_dir_all(&root)?;
        let db_path = root.join("notes.db");
        db::init(&db_path)?;
        let conn = db::open(&db_path)?;

        let unchanged_path = root.join("unchanged.md");
        fs::write(&unchanged_path, "same")?;
        let unchanged = file_document(&root, &unchanged_path)?;
        db::upsert_document(&conn, &unchanged)?;

        let modified_path = root.join("modified.md");
        fs::write(&modified_path, "current")?;
        let mut modified = file_document(&root, &modified_path)?;
        modified.body = "previous".to_owned();
        modified.size_bytes = "previous".len().try_into()?;
        modified.content_hash = blake3::hash(modified.body.as_bytes()).to_hex().to_string();
        db::upsert_document(&conn, &modified)?;

        let deleted = DocumentUpsert {
            path: "deleted.md".to_owned(),
            body: "deleted".to_owned(),
            size_bytes: 7,
            mtime_ns: 1,
            content_hash: blake3::hash(b"deleted").to_hex().to_string(),
            created_at: 1,
            updated_at: 1,
            synced_at: 1,
        };
        db::upsert_document(&conn, &deleted)?;

        fs::write(root.join("new.md"), "new")?;

        let summary = sync_dir(&root, &db_path)?;

        assert_eq!(
            summary,
            status::StatusSummary {
                new: 1,
                modified: 1,
                deleted: 1,
                unchanged: 1,
            }
        );

        let conn = db::open(&db_path)?;
        let new_body: String = conn.query_row(
            "SELECT body FROM documents WHERE path = 'new.md'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(new_body, "new");

        let modified_body: String = conn.query_row(
            "SELECT body FROM documents WHERE path = 'modified.md'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(modified_body, "current");

        let deleted_at: Option<i64> = conn
            .query_row(
                "SELECT deleted_at FROM documents WHERE path = 'deleted.md'",
                [],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        assert!(deleted_at.is_some());

        fs::remove_dir_all(root)?;

        Ok(())
    }

    fn file_document(root: &Path, path: &Path) -> Result<DocumentUpsert> {
        ingest::document_from_file(root, path, files::now_epoch_ns()?)
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();

        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }
}
