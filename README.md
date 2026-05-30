# mdlite

`mdlite` is a toy project: a CLI-first Markdown SQLite store.

## Why mdlite

The idea is to keep normal `.md` files as the editing source of truth, while
using SQLite + FTS5 as a fast indexed search store.

The local benchmarks behind this project suggest a practical split:

- `rg` is a great zero-index baseline, but repeated indexed body search is much
  faster with SQLite FTS5.
- warm random reads for small Markdown files can be faster from SQLite than from
  many separate files.
- whole-document writes are faster as direct filesystem overwrites, so editing
  should stay on the filesystem side.

These numbers come from
`/Users/akira.noda/dev/personal/sqlite-performance/small-files-io/bench-results-*.md`.
They are local toy-project benchmarks, not universal claims.

### Body Search: SQLite FTS5 vs File System + `rg`

Dataset: `10,000` documents, `4 KB` each.

| query | target | elapsed | matches |
| --- | --- | ---: | ---: |
| `latency` | `rg` over files | `0.187665s` | `9,920` |
| `latency` | SQLite `LIKE` | `0.075956s` | `9,920` |
| `latency` | SQLite FTS5 | `0.000809s` | `9,920` |
| `00009999` | `rg` over files | `0.112091s` | `1` |
| `00009999` | SQLite `LIKE` | `0.045543s` | `1` |
| `00009999` | SQLite FTS5 | `0.000847s` | `1` |

FTS5 made repeated body search much faster in this benchmark, but increased the
SQLite store size from about `46.8 MB` to `106.3 MB` for the `10,000 x 4 KB`
dataset. Building the FTS store took about `2.1s`.

### Random Reads

Dataset: `10,000` documents, `1,000` random reads, `5` repeats.

Representative warm-repeat results:

| document size | target | ops/sec | p50 | p95 |
| ---: | --- | ---: | ---: | ---: |
| `512 B` | files | `82,316.78` | `0.011583ms` | `0.016333ms` |
| `512 B` | SQLite | `416,399.48` | `0.002417ms` | `0.003125ms` |
| `4 KB` | files | `86,373.73` | `0.011416ms` | `0.012625ms` |
| `4 KB` | SQLite | `303,616.65` | `0.003334ms` | `0.004042ms` |
| `32 KB` | files | `68,254.29` | `0.014125ms` | `0.018458ms` |
| `32 KB` | SQLite | `120,042.24` | `0.008250ms` | `0.009166ms` |
| `256 KB` | files | `31,154.03` | `0.031792ms` | `0.035375ms` |
| `256 KB` | SQLite | `21,683.13` | `0.044250ms` | `0.050584ms` |

In these runs, warm random reads favored SQLite for small Markdown files up to
`32 KB`, while `256 KB` favored direct filesystem reads.

### Random Writes

Dataset: `10,000` documents, `4 KB` each, `1,000` whole-document updates.

| target | elapsed | updates/sec | MB/sec |
| --- | ---: | ---: | ---: |
| files | `0.110553s` | `9,045.40` | `37.05` |
| SQLite | `0.275423s` | `3,630.78` | `14.87` |

Direct filesystem overwrite was about `2.49x` faster than SQLite for this
whole-document update benchmark. That is why `mdlite` keeps filesystem files as
the editing workspace and uses SQLite mainly for indexed search and metadata.

## Usage

The current MVP stores each Markdown file as a whole body. It does not parse
frontmatter, titles, or tags.

Create or update the default database at `.mdstore/mdstore.db`:

```bash
cargo run --bin mdstore -- init
cargo run --bin mdstore -- sync notes
```

Search indexed Markdown:

```bash
cargo run --bin mdstore -- search SQLite
cargo run --bin mdstore -- search 'SQLite NEAR FTS5' --limit 10
```

Use an explicit database path:

```bash
cargo run --bin mdstore -- sync notes --db notes.db
cargo run --bin mdstore -- search SQLite --db notes.db
```

Check what would change without writing:

```bash
cargo run --bin mdstore -- status notes --db notes.db
```

Output:

```text
new: 1
modified: 0
deleted: 0
unchanged: 12
```

Inspect database health:

```bash
cargo run --bin mdstore -- doctor --db notes.db
```

Output:

```text
documents: 12
deleted: 0
fts_rows: 12
db_size_bytes: 73728
wal_size_bytes: 0
page_size: 4096
page_count: 18
integrity_check: ok
```

Initial bulk import is also available:

```bash
cargo run --bin mdstore -- import notes --db notes.db
```

`import` reads and hashes every Markdown file. For normal incremental work,
prefer `sync`.

## Commands

```bash
mdstore init [db]
mdstore import <dir> --db <db>
mdstore sync <dir> --db <db>
mdstore status <dir> --db <db>
mdstore search <query> --db <db> --limit 20
mdstore doctor --db <db>
```

When `--db` is omitted, commands use `.mdstore/mdstore.db`.

## Current Non-goals

- bidirectional sync
- conflict resolution
- file watching
- FUSE/filesystem mount
- Markdown AST parsing
- frontmatter/title/tag parsing
- normalized tag tables
