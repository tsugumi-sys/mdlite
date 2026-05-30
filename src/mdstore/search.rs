use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::params;

use crate::mdstore::db;

#[derive(Debug, PartialEq)]
pub struct SearchResult {
    pub path: String,
    pub score: f64,
}

pub fn search(db_path: &Path, query: &str, limit: u32) -> Result<Vec<SearchResult>> {
    let conn = db::open(db_path)?;
    let mut statement = conn
        .prepare(
            r#"
            SELECT d.path, bm25(documents_fts) AS score
            FROM documents_fts
            JOIN documents d ON d.id = documents_fts.rowid
            WHERE documents_fts MATCH ?1
              AND d.deleted_at IS NULL
            ORDER BY score
            LIMIT ?2
            "#,
        )
        .context("failed to prepare search query")?;

    let rows = statement
        .query_map(params![query, i64::from(limit)], |row| {
            Ok(SearchResult {
                path: row.get(0)?,
                score: row.get(1)?,
            })
        })
        .with_context(|| format!("failed to search for query {query:?}"))?;

    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("failed to read search results")
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::mdstore::db::DocumentUpsert;

    use super::*;

    #[test]
    fn searches_non_deleted_documents() -> Result<()> {
        let db_path = unique_temp_db_path();
        db::init(&db_path)?;
        let conn = db::open(&db_path)?;

        db::upsert_document(
            &conn,
            &DocumentUpsert {
                path: "sqlite.md".to_owned(),
                body: "sqlite full text search".to_owned(),
                size_bytes: 23,
                mtime_ns: 1,
                content_hash: "hash-1".to_owned(),
                created_at: 1,
                updated_at: 1,
                synced_at: 1,
            },
        )?;
        db::upsert_document(
            &conn,
            &DocumentUpsert {
                path: "other.md".to_owned(),
                body: "unrelated markdown".to_owned(),
                size_bytes: 18,
                mtime_ns: 1,
                content_hash: "hash-2".to_owned(),
                created_at: 1,
                updated_at: 1,
                synced_at: 1,
            },
        )?;

        let results = search(&db_path, "sqlite", 20)?;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "sqlite.md");

        std::fs::remove_file(db_path)?;

        Ok(())
    }

    fn unique_temp_db_path() -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();

        std::env::temp_dir().join(format!("mdlite-search-{nanos}.db"))
    }
}
