use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::mdstore::db::{self, DocumentUpsert};
use crate::mdstore::files;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImportSummary {
    pub imported: usize,
}

pub fn import_dir(dir: &Path, db_path: &Path) -> Result<ImportSummary> {
    if !dir.is_dir() {
        bail!("import path is not a directory: {}", dir.display());
    }

    db::init(db_path)?;
    let conn = db::open(db_path)?;
    let mut summary = ImportSummary::default();
    let synced_at = files::now_epoch_ns()?;

    for path in files::markdown_files(dir)? {
        let document = document_from_file(dir, &path, synced_at)?;
        db::upsert_document(&conn, &document)?;
        summary.imported += 1;
    }

    Ok(summary)
}

pub fn document_from_file(root: &Path, path: &Path, synced_at: i64) -> Result<DocumentUpsert> {
    let source = fs::read_to_string(path)
        .with_context(|| format!("failed to read markdown file {}", path.display()))?;
    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to read file metadata {}", path.display()))?;
    let mtime_ns = files::system_time_epoch_ns(
        metadata
            .modified()
            .with_context(|| format!("failed to read modified time for {}", path.display()))?,
    )?;
    let created_at = metadata
        .created()
        .ok()
        .and_then(files::system_time_epoch_ns_ok)
        .unwrap_or(mtime_ns);
    let updated_at = mtime_ns;
    let relative_path = files::relative_markdown_path(root, path)?;

    Ok(DocumentUpsert {
        path: relative_path,
        body: source.clone(),
        size_bytes: metadata.len().try_into().with_context(|| {
            format!("file is too large to store size as i64: {}", path.display())
        })?,
        mtime_ns,
        content_hash: blake3::hash(source.as_bytes()).to_hex().to_string(),
        created_at,
        updated_at,
        synced_at,
    })
}
