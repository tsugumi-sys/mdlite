use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use mdlite::mdstore::{db, doctor, ingest, search, status, sync};

#[derive(Debug, Parser)]
#[command(name = "mdstore")]
#[command(about = "Markdown SQLite store")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Initialize a Markdown store database.
    Init {
        /// SQLite database path. Defaults to .mdstore/mdstore.db.
        db: Option<PathBuf>,
    },
    /// Import Markdown files from a directory into the store.
    Import {
        /// Directory containing Markdown files.
        dir: PathBuf,
        /// SQLite database path. Defaults to .mdstore/mdstore.db.
        #[arg(long)]
        db: Option<PathBuf>,
    },
    /// Search imported Markdown with SQLite FTS5.
    Search {
        /// FTS5 query string.
        query: String,
        /// SQLite database path. Defaults to .mdstore/mdstore.db.
        #[arg(long)]
        db: Option<PathBuf>,
        /// Maximum number of results to print.
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Show filesystem vs store differences without writing to the database.
    Status {
        /// Directory containing Markdown files.
        dir: PathBuf,
        /// SQLite database path. Defaults to .mdstore/mdstore.db.
        #[arg(long)]
        db: Option<PathBuf>,
    },
    /// Sync filesystem Markdown changes into the store.
    Sync {
        /// Directory containing Markdown files.
        dir: PathBuf,
        /// SQLite database path. Defaults to .mdstore/mdstore.db.
        #[arg(long)]
        db: Option<PathBuf>,
    },
    /// Show database health and store statistics.
    Doctor {
        /// SQLite database path. Defaults to .mdstore/mdstore.db.
        #[arg(long)]
        db: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Init { db } => {
            let db_path = db.unwrap_or_else(db::default_db_path);
            db::init(&db_path)?;
            println!("initialized {}", db_path.display());
        }
        Command::Import { dir, db } => {
            let db_path = db.unwrap_or_else(db::default_db_path);
            let summary = ingest::import_dir(&dir, &db_path)?;
            println!("imported: {}", summary.imported);
        }
        Command::Search { query, db, limit } => {
            let db_path = db.unwrap_or_else(db::default_db_path);
            for result in search::search(&db_path, &query, limit)? {
                println!("{}", result.path);
                println!("  score: {}", result.score);
            }
        }
        Command::Status { dir, db } => {
            let db_path = db.unwrap_or_else(db::default_db_path);
            let summary = status::status_dir(&dir, &db_path)?;
            println!("new: {}", summary.new);
            println!("modified: {}", summary.modified);
            println!("deleted: {}", summary.deleted);
            println!("unchanged: {}", summary.unchanged);
        }
        Command::Sync { dir, db } => {
            let db_path = db.unwrap_or_else(db::default_db_path);
            let summary = sync::sync_dir(&dir, &db_path)?;
            println!("new: {}", summary.new);
            println!("modified: {}", summary.modified);
            println!("deleted: {}", summary.deleted);
            println!("unchanged: {}", summary.unchanged);
        }
        Command::Doctor { db } => {
            let db_path = db.unwrap_or_else(db::default_db_path);
            let report = doctor::doctor(&db_path)?;
            println!("documents: {}", report.documents);
            println!("deleted: {}", report.deleted);
            println!("fts_rows: {}", report.fts_rows);
            println!("db_size_bytes: {}", report.db_size_bytes);
            println!("wal_size_bytes: {}", report.wal_size_bytes);
            println!("page_size: {}", report.page_size);
            println!("page_count: {}", report.page_count);
            println!("integrity_check: {}", report.integrity_check);
        }
    }

    Ok(())
}
