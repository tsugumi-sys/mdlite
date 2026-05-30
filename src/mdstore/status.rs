use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::mdstore::db;
use crate::mdstore::files;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct StatusSummary {
    pub new: usize,
    pub modified: usize,
    pub deleted: usize,
    pub unchanged: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub struct FileChange {
    pub relative_path: String,
    pub path: PathBuf,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct StatusDiff {
    pub new: Vec<FileChange>,
    pub modified: Vec<FileChange>,
    pub deleted: Vec<String>,
    pub unchanged: Vec<String>,
}

impl StatusDiff {
    pub fn summary(&self) -> StatusSummary {
        StatusSummary {
            new: self.new.len(),
            modified: self.modified.len(),
            deleted: self.deleted.len(),
            unchanged: self.unchanged.len(),
        }
    }
}

pub fn status_dir(dir: &Path, db_path: &Path) -> Result<StatusSummary> {
    Ok(diff_dir(dir, db_path)?.summary())
}

pub fn diff_dir(dir: &Path, db_path: &Path) -> Result<StatusDiff> {
    if !dir.is_dir() {
        bail!("status path is not a directory: {}", dir.display());
    }

    let conn = db::open_read_only(db_path)?;
    let db_documents = db::active_document_metadata(&conn)?
        .into_iter()
        .map(|document| (document.path.clone(), document))
        .collect::<HashMap<_, _>>();

    let mut seen_paths = HashSet::new();
    let mut diff = StatusDiff::default();

    for path in files::markdown_files(dir)? {
        let relative_path = files::relative_markdown_path(dir, &path)?;
        seen_paths.insert(relative_path.clone());

        let Some(db_document) = db_documents.get(&relative_path) else {
            diff.new.push(FileChange {
                relative_path,
                path,
            });
            continue;
        };

        let metadata = fs::metadata(&path)
            .with_context(|| format!("failed to read file metadata {}", path.display()))?;
        let size_bytes: i64 = metadata.len().try_into().with_context(|| {
            format!("file is too large to store size as i64: {}", path.display())
        })?;
        let mtime_ns =
            files::system_time_epoch_ns(metadata.modified().with_context(|| {
                format!("failed to read modified time for {}", path.display())
            })?)?;

        if size_bytes == db_document.size_bytes && mtime_ns == db_document.mtime_ns {
            diff.unchanged.push(relative_path);
            continue;
        }

        if content_hash(&path)? == db_document.content_hash {
            diff.unchanged.push(relative_path);
        } else {
            diff.modified.push(FileChange {
                relative_path,
                path,
            });
        }
    }

    diff.deleted = db_documents
        .values()
        .filter(|document| !seen_paths.contains(&document.path))
        .map(|document| document.path.clone())
        .collect();

    Ok(diff)
}

fn content_hash(path: &Path) -> Result<String> {
    let body = fs::read(path)
        .with_context(|| format!("failed to read markdown file {}", path.display()))?;
    Ok(blake3::hash(&body).to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use rusqlite::params;

    use crate::mdstore::db::DocumentUpsert;

    use super::*;

    #[test]
    fn classifies_new_modified_deleted_and_unchanged_documents() -> Result<()> {
        let root = unique_temp_dir("mdlite-status");
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

        let same_hash_path = root.join("same-hash.md");
        fs::write(&same_hash_path, "same hash")?;
        let mut same_hash = file_document(&root, &same_hash_path)?;
        same_hash.mtime_ns -= 1;
        db::upsert_document(&conn, &same_hash)?;

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

        let summary = status_dir(&root, &db_path)?;

        assert_eq!(
            summary,
            StatusSummary {
                new: 1,
                modified: 1,
                deleted: 1,
                unchanged: 2,
            }
        );

        fs::remove_dir_all(root)?;

        Ok(())
    }

    #[test]
    fn ignores_soft_deleted_documents() -> Result<()> {
        let root = unique_temp_dir("mdlite-status-soft-delete");
        fs::create_dir_all(&root)?;
        let db_path = root.join("notes.db");
        db::init(&db_path)?;
        let conn = db::open(&db_path)?;
        let deleted = DocumentUpsert {
            path: "already-deleted.md".to_owned(),
            body: "deleted".to_owned(),
            size_bytes: 7,
            mtime_ns: 1,
            content_hash: blake3::hash(b"deleted").to_hex().to_string(),
            created_at: 1,
            updated_at: 1,
            synced_at: 1,
        };
        db::upsert_document(&conn, &deleted)?;
        conn.execute(
            "UPDATE documents SET deleted_at = ?1 WHERE path = ?2",
            params![2_i64, "already-deleted.md"],
        )?;

        let summary = status_dir(&root, &db_path)?;

        assert_eq!(summary, StatusSummary::default());

        fs::remove_dir_all(root)?;

        Ok(())
    }

    fn file_document(root: &Path, path: &Path) -> Result<DocumentUpsert> {
        let body = fs::read_to_string(path)?;
        let metadata = fs::metadata(path)?;
        let mtime_ns = files::system_time_epoch_ns(metadata.modified()?)?;
        Ok(DocumentUpsert {
            path: files::relative_markdown_path(root, path)?,
            body: body.clone(),
            size_bytes: metadata.len().try_into()?,
            mtime_ns,
            content_hash: blake3::hash(body.as_bytes()).to_hex().to_string(),
            created_at: mtime_ns,
            updated_at: mtime_ns,
            synced_at: mtime_ns,
        })
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();

        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }
}
