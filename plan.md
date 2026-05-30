# Markdown SQLite Store MVP Plan

## 目的

Markdown を SQLite に保存し、FTS5 で高速検索できる Markdown 専用 store を作る。

最初から filesystem API/FUSE は作らず、MVP は CLI-first にする。

狙い:

- Markdown 検索を高速化する
- 小さめ Markdown の I/O 固定費を減らす
- 普通のエディタでは `.md` ファイルとして編集できる状態を維持する
- SQLite store は検索・メタデータ管理・同期先として使う

## MVP の基本方針

Filesystem を編集用 workspace、SQLite を検索用/indexed store として扱う。

```text
./notes/*.md
  -> editor で編集
  -> mdstore sync ./notes
  -> SQLite documents + FTS5 に反映
  -> mdstore search "keyword"
```

MVP では双方向同期はやらない。

```text
MVP:
  filesystem -> sqlite

Later:
  sqlite -> filesystem
  conflict detection
  FUSE/filesystem API
```

## ベンチ結果からの判断

これまでのベンチで見えたこと:

- 小さい Markdown の random read は SQLite が有利になりやすい
- warm cache では SQLite がかなり速い
- 512B〜32KB の Markdown では SQLite が強い
- 256KB くらいになると File System が有利になり始める
- 本文検索は SQLite FTS5 がかなり速い
- FTS5 は速いが DB サイズと index 更新コストが増える
- 単純な whole-document update は File System direct overwrite のほうが速い

MVP の設計判断:

- 編集そのものは File System workspace に任せる
- 検索と一覧メタデータは SQLite に任せる
- `sync` で変更された Markdown だけ SQLite/FTS5 に反映する

## CLI

バイナリ名の仮案:

```bash
mdstore
```

MVP コマンド:

```bash
mdstore init <db>
mdstore import <dir> --db <db>
mdstore sync <dir> --db <db>
mdstore search <query> --db <db>
mdstore status <dir> --db <db>
mdstore doctor --db <db>
```

後回し:

```bash
mdstore export <dir> --db <db>
mdstore read <path> --db <db>
mdstore write <path> --db <db>
mdstore mount <dir> --db <db>
```

## SQLite Schema

初期 schema:

```sql
CREATE TABLE documents (
  id INTEGER PRIMARY KEY,
  path TEXT NOT NULL UNIQUE,
  title TEXT NOT NULL,
  tags TEXT NOT NULL,
  body TEXT NOT NULL,
  size_bytes INTEGER NOT NULL,
  mtime_ns INTEGER NOT NULL,
  content_hash TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  synced_at INTEGER NOT NULL,
  deleted_at INTEGER
);

CREATE INDEX idx_documents_updated_at ON documents(updated_at);
CREATE INDEX idx_documents_synced_at ON documents(synced_at);
CREATE INDEX idx_documents_deleted_at ON documents(deleted_at);
```

FTS5:

```sql
CREATE VIRTUAL TABLE documents_fts USING fts5(
  path UNINDEXED,
  title,
  body
);
```

FTS 同期は trigger で行う。

```sql
CREATE TRIGGER documents_ai AFTER INSERT ON documents BEGIN
  INSERT INTO documents_fts(rowid, path, title, body)
  VALUES (new.id, new.path, new.title, new.body);
END;

CREATE TRIGGER documents_ad AFTER DELETE ON documents BEGIN
  INSERT INTO documents_fts(documents_fts, rowid, path, title, body)
  VALUES ('delete', old.id, old.path, old.title, old.body);
END;

CREATE TRIGGER documents_au AFTER UPDATE ON documents BEGIN
  INSERT INTO documents_fts(documents_fts, rowid, path, title, body)
  VALUES ('delete', old.id, old.path, old.title, old.body);
  INSERT INTO documents_fts(rowid, path, title, body)
  VALUES (new.id, new.path, new.title, new.body);
END;
```

## Metadata

Markdown frontmatter から最低限これを取る。

```yaml
---
title: Example
tags: [sqlite, markdown]
created_at: 2026-01-01T00:00:00Z
updated_at: 2026-01-02T00:00:00Z
---
```

MVP の扱い:

- `title` がなければ最初の `# heading` を使う
- heading もなければ filename を使う
- `tags` がなければ空
- `created_at` / `updated_at` がなければ file metadata から補完
- frontmatter parser は最初は簡易実装でよい

## Sync Design

MVP の sync は片方向。

```text
filesystem -> sqlite
```

差分判定:

1. filesystem を walk
2. `.md` のみ対象
3. path / size / mtime_ns を取得
4. SQLite の既存 document と比較
5. size と mtime_ns が同じなら skip
6. 違う場合だけ file body を読む
7. content_hash を計算
8. hash が違う場合だけ upsert
9. SQLite に存在し、filesystem に存在しない path は `deleted_at` を入れる

重要:

- 毎回全ファイルの hash は計算しない
- hash は size/mtime が変わった時だけ計算する
- 削除は物理削除ではなく soft delete から始める

## Hash

MVP は SHA-256 または BLAKE3 を使う。

候補:

- `sha2`: 標準的
- `blake3`: 高速

toy project なら `blake3` がよい。

保存値:

```text
content_hash: hex string
```

## Search

検索は FTS5 を使う。

```bash
mdstore search "sqlite performance" --db notes.db
```

初期出力:

```text
path/to/doc.md
  title: SQLite Performance
  score: ...
```

MVP query:

```sql
SELECT d.path, d.title, bm25(documents_fts) AS score
FROM documents_fts
JOIN documents d ON d.id = documents_fts.rowid
WHERE documents_fts MATCH ?
  AND d.deleted_at IS NULL
ORDER BY score
LIMIT ?;
```

後で追加:

- snippet
- highlight
- tag filter
- updated_at sort
- path prefix filter

## Status

`status` は dry-run sync のようなコマンド。

```bash
mdstore status ./notes --db notes.db
```

出力例:

```text
new: 12
modified: 4
deleted: 2
unchanged: 984
```

実装上は `sync` と同じ差分検出を使い、DB 書き込みだけしない。

## Import

`import` は初回投入。

```bash
mdstore import ./notes --db notes.db
```

挙動:

- DB がなければ schema 作成
- `.md` を全件読む
- frontmatter parse
- hash 計算
- documents に insert
- FTS は trigger で更新

`sync` でも初回投入はできるようにしてよいが、MVP では `import` を明示コマンドとして用意する。

## Doctor

`doctor` は DB 状態を確認する。

```bash
mdstore doctor --db notes.db
```

確認項目:

- documents 件数
- deleted 件数
- FTS table の件数
- DB size
- WAL size
- page_size
- page_count
- integrity_check

## MVP Implementation Steps

### Step 1: 新しい CLI 骨格

既存ベンチツールとは別にするか、同じ repo 内で `src/bin/mdstore.rs` として作る。

推奨:

```text
src/bin/mdstore.rs
src/mdstore/
  mod.rs
  db.rs
  markdown.rs
  sync.rs
  search.rs
```

### Step 2: `init`

- DB 作成
- schema 作成
- FTS5 作成
- triggers 作成

### Step 3: `import`

- directory walk
- Markdown parse
- hash 計算
- documents insert
- FTS trigger 動作確認

### Step 4: `search`

- FTS5 query
- path/title/score を出力

### Step 5: `status`

- filesystem と DB の差分検出
- new/modified/deleted/unchanged を表示

### Step 6: `sync`

- new insert
- modified update
- missing file を soft delete
- sync 結果を表示

### Step 7: `doctor`

- 件数
- DB サイズ
- FTS 件数
- integrity_check

## Non-goals for MVP

MVP ではやらない:

- FUSE / filesystem mount
- 双方向 sync
- conflict resolution
- concurrent writer support
- file watcher daemon
- atomic export
- Markdown AST parsing
- tag 正規化テーブル
- frontmatter 完全YAML対応

## Open Questions

- DB を workspace 内に置くか、外に置くか
- path は workspace root からの relative path でよいか
- deleted document はいつ purge するか
- sync 時に unknown frontmatter を保持するか
- FTS tokenizer をどうするか
- 日本語検索をどう扱うか

## First MVP Target

最初の完成条件:

```bash
mdstore init notes.db
mdstore import ./notes --db notes.db
mdstore search "sqlite" --db notes.db
mdstore status ./notes --db notes.db
mdstore sync ./notes --db notes.db
mdstore doctor --db notes.db
```

これで「普通のエディタで Markdown を編集し、SQLite/FTS5 で高速検索する」最小体験が成立する。

## 要件

- Rustで構築。
- init -> default db + gitignoreがあった方が良さそう。もしくはdb pathをuserのspaceにおくとか。
- コマンドを一つずつ作っていこう。コードを理解したい, レビューしたい。
