use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags, params};

pub const DEFAULT_DB_DIR: &str = ".mdstore";
pub const DEFAULT_DB_FILE: &str = "mdstore.db";

pub fn default_db_path() -> PathBuf {
    PathBuf::from(DEFAULT_DB_DIR).join(DEFAULT_DB_FILE)
}

pub fn init(db_path: &Path) -> Result<()> {
    if let Some(parent) = db_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create database directory {}", parent.display()))?;
    }

    let conn = Connection::open(db_path)
        .with_context(|| format!("failed to open database {}", db_path.display()))?;

    migrate_legacy_title_tags_schema(&conn)?;

    conn.execute_batch(SCHEMA).with_context(|| {
        format!(
            "failed to initialize database schema in {}",
            db_path.display()
        )
    })?;

    Ok(())
}

pub fn open(db_path: &Path) -> Result<Connection> {
    Connection::open(db_path)
        .with_context(|| format!("failed to open database {}", db_path.display()))
}

pub fn open_read_only(db_path: &Path) -> Result<Connection> {
    Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("failed to open database read-only {}", db_path.display()))
}

fn migrate_legacy_title_tags_schema(conn: &Connection) -> Result<()> {
    if !has_documents_column(conn, "title")? && !has_documents_column(conn, "tags")? {
        return Ok(());
    }

    conn.execute_batch(
        r#"
        DROP TRIGGER IF EXISTS documents_ai;
        DROP TRIGGER IF EXISTS documents_ad;
        DROP TRIGGER IF EXISTS documents_au;
        DROP TABLE IF EXISTS documents_fts;
        DROP INDEX IF EXISTS idx_documents_updated_at;
        DROP INDEX IF EXISTS idx_documents_synced_at;
        DROP INDEX IF EXISTS idx_documents_deleted_at;

        ALTER TABLE documents RENAME TO documents_legacy_title_tags;

        CREATE TABLE documents (
          id INTEGER PRIMARY KEY,
          path TEXT NOT NULL UNIQUE,
          body TEXT NOT NULL,
          size_bytes INTEGER NOT NULL,
          mtime_ns INTEGER NOT NULL,
          content_hash TEXT NOT NULL,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL,
          synced_at INTEGER NOT NULL,
          deleted_at INTEGER
        );

        INSERT INTO documents (
          id,
          path,
          body,
          size_bytes,
          mtime_ns,
          content_hash,
          created_at,
          updated_at,
          synced_at,
          deleted_at
        )
        SELECT
          id,
          path,
          body,
          size_bytes,
          mtime_ns,
          content_hash,
          created_at,
          updated_at,
          synced_at,
          deleted_at
        FROM documents_legacy_title_tags;

        DROP TABLE documents_legacy_title_tags;
        "#,
    )
    .context("failed to migrate legacy title/tags schema")?;

    Ok(())
}

fn has_documents_column(conn: &Connection, column_name: &str) -> Result<bool> {
    let mut statement = conn
        .prepare("SELECT name FROM pragma_table_info('documents')")
        .context("failed to inspect documents table")?;
    let mut rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .context("failed to read documents table columns")?;

    rows.try_fold(false, |found, name| -> rusqlite::Result<bool> {
        let name = name?;
        Ok(found || name == column_name)
    })
    .context("failed to inspect documents table columns")
}

#[derive(Debug)]
pub struct DocumentUpsert {
    pub path: String,
    pub body: String,
    pub size_bytes: i64,
    pub mtime_ns: i64,
    pub content_hash: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub synced_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentMetadata {
    pub path: String,
    pub size_bytes: i64,
    pub mtime_ns: i64,
    pub content_hash: String,
}

pub fn active_document_metadata(conn: &Connection) -> Result<Vec<DocumentMetadata>> {
    let mut statement = conn
        .prepare(
            r#"
            SELECT path, size_bytes, mtime_ns, content_hash
            FROM documents
            WHERE deleted_at IS NULL
            "#,
        )
        .context("failed to prepare active document metadata query")?;

    let rows = statement
        .query_map([], |row| {
            Ok(DocumentMetadata {
                path: row.get(0)?,
                size_bytes: row.get(1)?,
                mtime_ns: row.get(2)?,
                content_hash: row.get(3)?,
            })
        })
        .context("failed to query active document metadata")?;

    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to read active document metadata")
}

pub fn upsert_document(conn: &Connection, document: &DocumentUpsert) -> Result<()> {
    conn.execute(
        r#"
        INSERT INTO documents (
          path,
          body,
          size_bytes,
          mtime_ns,
          content_hash,
          created_at,
          updated_at,
          synced_at,
          deleted_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL)
        ON CONFLICT(path) DO UPDATE SET
          body = excluded.body,
          size_bytes = excluded.size_bytes,
          mtime_ns = excluded.mtime_ns,
          content_hash = excluded.content_hash,
          updated_at = excluded.updated_at,
          synced_at = excluded.synced_at,
          deleted_at = NULL
        "#,
        params![
            document.path,
            document.body,
            document.size_bytes,
            document.mtime_ns,
            document.content_hash,
            document.created_at,
            document.updated_at,
            document.synced_at,
        ],
    )
    .with_context(|| format!("failed to upsert document {}", document.path))?;

    Ok(())
}

pub fn soft_delete_document(conn: &Connection, path: &str, deleted_at: i64) -> Result<()> {
    conn.execute(
        r#"
        UPDATE documents
        SET deleted_at = ?1,
            synced_at = ?1
        WHERE path = ?2
          AND deleted_at IS NULL
        "#,
        params![deleted_at, path],
    )
    .with_context(|| format!("failed to soft delete document {path}"))?;

    Ok(())
}

const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS documents (
  id INTEGER PRIMARY KEY,
  path TEXT NOT NULL UNIQUE,
  body TEXT NOT NULL,
  size_bytes INTEGER NOT NULL,
  mtime_ns INTEGER NOT NULL,
  content_hash TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  synced_at INTEGER NOT NULL,
  deleted_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_documents_updated_at ON documents(updated_at);
CREATE INDEX IF NOT EXISTS idx_documents_synced_at ON documents(synced_at);
CREATE INDEX IF NOT EXISTS idx_documents_deleted_at ON documents(deleted_at);

CREATE VIRTUAL TABLE IF NOT EXISTS documents_fts USING fts5(
  path UNINDEXED,
  body
);

DROP TRIGGER IF EXISTS documents_ai;
DROP TRIGGER IF EXISTS documents_ad;
DROP TRIGGER IF EXISTS documents_au;

CREATE TRIGGER documents_ai AFTER INSERT ON documents BEGIN
  INSERT INTO documents_fts(rowid, path, body)
  VALUES (new.id, new.path, new.body);
END;

CREATE TRIGGER documents_ad AFTER DELETE ON documents BEGIN
  DELETE FROM documents_fts WHERE rowid = old.id;
END;

CREATE TRIGGER documents_au AFTER UPDATE ON documents BEGIN
  DELETE FROM documents_fts WHERE rowid = old.id;
  INSERT INTO documents_fts(rowid, path, body)
  VALUES (new.id, new.path, new.body);
END;
"#;

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use rusqlite::{Connection, params};

    use super::*;

    #[test]
    fn init_creates_schema_and_fts_triggers() -> Result<()> {
        let db_path = unique_temp_db_path();

        init(&db_path)?;

        let conn = Connection::open(&db_path)?;
        let document_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))?;
        assert_eq!(document_count, 0);

        conn.execute(
            r#"
            INSERT INTO documents (
              path,
              body,
              size_bytes,
              mtime_ns,
              content_hash,
              created_at,
              updated_at,
              synced_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                "note.md",
                "Markdown content about sqlite search.",
                37,
                1,
                "hash",
                1,
                1,
                1,
            ],
        )?;

        let fts_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM documents_fts WHERE documents_fts MATCH 'sqlite'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(fts_count, 1);

        std::fs::remove_file(db_path)?;

        Ok(())
    }

    fn unique_temp_db_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();

        std::env::temp_dir().join(format!("mdlite-init-{nanos}.db"))
    }
}
