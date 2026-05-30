use std::path::Path;

use anyhow::{Context, Result};

use crate::mdstore::db;

#[derive(Debug, PartialEq, Eq)]
pub struct DoctorReport {
    pub documents: i64,
    pub deleted: i64,
    pub fts_rows: i64,
    pub db_size_bytes: u64,
    pub wal_size_bytes: u64,
    pub page_size: i64,
    pub page_count: i64,
    pub integrity_check: String,
}

pub fn doctor(db_path: &Path) -> Result<DoctorReport> {
    let conn = db::open_read_only(db_path)?;

    Ok(DoctorReport {
        documents: count(
            &conn,
            "SELECT COUNT(*) FROM documents WHERE deleted_at IS NULL",
        )
        .context("failed to count active documents")?,
        deleted: count(
            &conn,
            "SELECT COUNT(*) FROM documents WHERE deleted_at IS NOT NULL",
        )
        .context("failed to count deleted documents")?,
        fts_rows: count(&conn, "SELECT COUNT(*) FROM documents_fts")
            .context("failed to count FTS rows")?,
        db_size_bytes: file_size(db_path)?,
        wal_size_bytes: file_size(&wal_path(db_path))?,
        page_size: pragma_i64(&conn, "PRAGMA page_size").context("failed to read page_size")?,
        page_count: pragma_i64(&conn, "PRAGMA page_count").context("failed to read page_count")?,
        integrity_check: pragma_string(&conn, "PRAGMA integrity_check")
            .context("failed to run integrity_check")?,
    })
}

fn count(conn: &rusqlite::Connection, query: &str) -> Result<i64> {
    Ok(conn.query_row(query, [], |row| row.get(0))?)
}

fn pragma_i64(conn: &rusqlite::Connection, query: &str) -> Result<i64> {
    Ok(conn.query_row(query, [], |row| row.get(0))?)
}

fn pragma_string(conn: &rusqlite::Connection, query: &str) -> Result<String> {
    Ok(conn.query_row(query, [], |row| row.get(0))?)
}

fn file_size(path: &Path) -> Result<u64> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => {
            Err(error).with_context(|| format!("failed to read file metadata {}", path.display()))
        }
    }
}

fn wal_path(db_path: &Path) -> std::path::PathBuf {
    let mut wal_path = db_path.as_os_str().to_owned();
    wal_path.push("-wal");
    wal_path.into()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::mdstore::db::DocumentUpsert;

    use super::*;

    #[test]
    fn reports_database_health_and_counts() -> Result<()> {
        let db_path = unique_temp_db_path();
        db::init(&db_path)?;
        let conn = db::open(&db_path)?;

        db::upsert_document(
            &conn,
            &DocumentUpsert {
                path: "active.md".to_owned(),
                body: "sqlite body".to_owned(),
                size_bytes: 11,
                mtime_ns: 1,
                content_hash: blake3::hash(b"sqlite body").to_hex().to_string(),
                created_at: 1,
                updated_at: 1,
                synced_at: 1,
            },
        )?;
        db::upsert_document(
            &conn,
            &DocumentUpsert {
                path: "deleted.md".to_owned(),
                body: "deleted body".to_owned(),
                size_bytes: 12,
                mtime_ns: 1,
                content_hash: blake3::hash(b"deleted body").to_hex().to_string(),
                created_at: 1,
                updated_at: 1,
                synced_at: 1,
            },
        )?;
        db::soft_delete_document(&conn, "deleted.md", 2)?;
        drop(conn);

        let report = doctor(&db_path)?;

        assert_eq!(report.documents, 1);
        assert_eq!(report.deleted, 1);
        assert_eq!(report.fts_rows, 2);
        assert!(report.db_size_bytes > 0);
        assert!(report.page_size > 0);
        assert!(report.page_count > 0);
        assert_eq!(report.integrity_check, "ok");

        std::fs::remove_file(db_path)?;

        Ok(())
    }

    fn unique_temp_db_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();

        std::env::temp_dir().join(format!("mdlite-doctor-{nanos}.db"))
    }
}
