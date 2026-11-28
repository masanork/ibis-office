# JetEngine 系データベース（Access / ESE / OST）× Turso 統合設計（RFC）

- 対象リポジトリ: `sorane-dev/docs`（`ibis-doc-model`, `ibis-doc-service`, `ibis-mcp`）
- 統合先ブランチ: `sorane`
- ステータス: Implemented（Access / ESE / Outlook OST-PST × Turso × MCP 統合完了）

## 1. 背景と目的

組織や個人が Microsoft 環境から **Google Workspace** やオープンなデータ基盤へ移行する際、以下の 2 つが実務上の大きな障壁となっている。

1. **Microsoft Access（`.mdb` / `.accdb`）資産の継承問題**
   - 業務データだけでなく、テーブル間のリレーションシップ、保存済みクエリ、フォーム／レポート定義が独自バイナリに閉じ込められており、Google Sheets / AppSheet / BigQuery 等への移行時に構造の棚卸しと再設計が高コストになる。
2. **ローカル業務データ資産（Access / ESE / Outlook `.ost`・`.pst`）への Coding Agent からのアクセス困難**
   - Windows/macOS/Linux 上の Coding Agent（Claude Code, Codex 等）やデスクトップ Assistant が、ODBC ドライバや Microsoft Access / Outlook 本体なしで、これらのバイナリ資産を安全かつ統一的に SQL クエリ・JOIN・集計できる環境が不足している。

本設計では、`ibis-doc-model` に組み込まれている **Turso（`turso_core` / `turso_parser`）** と **`ibis-mcp` / `ibis-doc-service`** の既存パイプラインを拡張し、Pure-Rust で JetEngine 系および関連バイナリ DB を読み取り、SQL クエリ・クロスフォーマット JOIN・Excel（Google Sheets 互換）出力・仕様書生成までを一気通貫で実現する。

---

## 2. 対象フォーマットの整理と Pure-Rust Crate 選定

技術的に混同されやすい 3 つの系統を明確に分離して扱う。

1. **JET Red / ACE（Microsoft Access: `.mdb`, `.accdb`）**
   - Jet 3 (Access 97), Jet 4 (Access 2000–2003), ACE 12–17 (Access 2007–2019+)。
   - リレーショナルテーブルに加え、`MSysObjects` / `MSysQueries` / `MSysRelationships` 等のシステムテーブルに設計資産を保持する。
2. **JET Blue / ESE（Extensible Storage Engine: `.edb`, `.dat`, `.sdb`）**
   - Exchange Server メールボックスストア（`.edb`）、Windows Search (`Windows.edb`)、Active Directory (`NTDS.dit`)、SRUM (`SRUDB.dat`)、WebCache (`WebCacheV01.dat`) 等の B-Tree ベース非 SQL ストレージ。
3. **Outlook オフライン／個人フォルダ（`.ost`, `.pst`）**
   - ※ Outlook クライアントの `.ost` / `.pst` は、実体は JET Blue (ESE) ではなく **MS-PST / NDB（Node Database / PFF）** 形式である（Exchange Server 側の `.edb` や一部の旧 Windows ストアが ESE）。本設計では両方を視野に入れ、パーサー層を明確に分離する。

### ライセンス制約（`deny.toml` 準拠）と採用候補

`deny.toml` により、商用ライセンスと両立しないコピーレフト（GPL / LGPL / AGPL）の依存クレートは禁止されている。したがって C 版 `mdbtools`（LGPL）やその直訳クレートは採用せず、**MIT / Apache-2.0 の Pure-Rust クレート**を採用する。

| 系統 | 拡張子 | 採用候補 Crate | ライセンス | 特徴・採用方針 |
| :--- | :--- | :--- | :--- | :--- |
| **JET Red / ACE** | `.mdb`, `.accdb` | `jetdb` | `MIT OR Apache-2.0` | **Phase 1 主採用**。Jet3–ACE17 読取専用、Pure Rust・C 依存なし。 |
| **JET Red / ACE（書込検討）** | `.mdb`, `.accdb` | `datacraft-jet` | `MIT OR Apache-2.0` | Phase 3 で評価。Jet4/ACE 書込やフォーム定義移行が必要な場合に検討。 |
| **JET Blue / ESE** | `.edb`, `.dat`, `.sdb` | `ese-core` | `Apache-2.0` | **Phase 2 採用**。依存 3 個（`thiserror`, `memmap2`, `serde`）、ファジング済み・読取専用。 |
| **Outlook NDB/PFF** | `.ost`, `.pst` | `outlook-pst` (`outlook-pst-rw`) | `MIT` | **Phase 2 採用**。MS-PST 公開仕様に基づく Microsoft 公式クリーンルーム実装。 |

---

## 3. 全体アーキテクチャ

既存の `crates/ibis-doc-model/src/query/sqlite_source.rs` にある `DiskDataset` と `SqliteQuerySession`（Turso インメモリ / 一時ディスク DB）を中核として統合する。

```text
  [.mdb / .accdb] ──▶ jet_source.rs (jetdb) ──────┐
  [.edb / .dat]   ──▶ ese_source.rs (ese-core) ───┼─▶ DiskDataset::visit()
  [.ost / .pst]   ──▶ pst_source.rs (outlook-pst) ┘         │
  [CSV / Parquet / Arrow / JSONL / Avro / SQLite] ──────────┘
                                                            ▼
                                            SqliteQuerySession (turso_core)
                                             ├─ In-memory / Temp-disk staging
                                             ├─ Optional Turso .sqlite cache
                                             └─ Read-only SELECT / WITH / JOIN
                                                            │
                    ┌───────────────────────────────────────┴───────────────────────────────────────┐
                    ▼                                                                               ▼
       ibis-doc-service / ibis-mcp                                                    Google Workspace 移行出力
  ├─ sqlite_describe_datasets / sqlite_query                                     ├─ .xlsx (Sheets 互換) + Pivot / QueryReport
  ├─ jet_inspect_schema / ese_list_tables / pst_list_folders                     ├─ Mermaid ER図 + Markdown 仕様書
  └─ Coding Agent (Claude Code / Codex) & Desktop Assistant                      └─ 保存済みクエリ SQL 抽出・変換支援
```

### 3.1 `ibis-doc-model/src/query/` への統合

`crates/ibis-doc-model/src/query.rs` の各ソースモジュール（`parquet_source.rs`, `avro_source.rs`, `sqlite_source.rs` 等）と同列に以下を追加する（Desktop ターゲット `cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))` 配下）。

1. **`jet_source.rs`（Access `.mdb` / `.accdb`）**
   - `jet_tables(path)` / `jet_inspect_schema(path)`: ユーザーテーブル一覧、各カラム名・型・PK/FK・インデックス、および `MSysObjects` / `MSysQueries` / `MSysRelationships` から取得可能なリレーション・保存済みクエリメタデータを返す。
   - `visit_jet_table(path, table, selected_columns, schema_only, visit)`: `DiskDataset` 経由で `SqliteQuerySession` へストリーミング投入する。
2. **`ese_source.rs`（ESE / JET Blue `.edb`, `.dat`, `.sdb`）**
   - `ese_tables(path)` / `ese_columns(path, table)` / `visit_ese_table(...)`: `ese-core` のカタログと B-Tree リーフレコード走査を `QColumn` / `CellValue` 列へ変換する。
3. **`pst_source.rs`（Outlook `.ost`, `.pst`）**
   - メールストアは単一テーブルではないため、正規化された仮想テーブル群として射影する：
     - `messages`（`entry_id`, `folder_path`, `subject`, `sender`, `recipients`, `sent_at`, `received_at`, `body_text`, `has_attachments`）
     - `folders`（`folder_id`, `parent_id`, `name`, `path`, `item_count`）
     - `contacts` / `appointments`（プロパティタグに基づく射影）

### 3.2 データ型マッピング（Access JET → `QType` / Turso）

| Access / JET 型 | `QType` | Turso (SQLite) 表現 | 備考 |
| :--- | :--- | :--- | :--- |
| Text / Memo (Long Text) | `Text` | `TEXT` | UTF-8 変換。Jet3/4 のコードページ（CP932 Shift_JIS 等）を考慮。 |
| Byte / Integer / Long / AutoNumber | `Int32` / `Whole` | `INTEGER` | 精度欠落なし。 |
| Single / Double | `Decimal` | `REAL` | IEEE 754 浮動小数点。 |
| Currency | `Currency` | `REAL` or `TEXT` | 厳密な固定小数点（10^-4）が必要な場合は `TEXT` 保持オプションを用意。 |
| Numeric / Decimal (17-byte) | `Text` / `Decimal` | `TEXT` | 28 桁精度保全のため SQLite 比較/集計方針と整合させる。 |
| DateTime | `DateTime` | `TEXT` (ISO-8601) / Serial | 既存 `sqlite_source` / `QType::DateTime` の表現に統一。 |
| Yes/No (Boolean) | `Logical` | `INTEGER` (`0`/`1`) | `QType::Logical` と整合。 |
| OLE Object / Binary / Attachment | `Text` (要約) | `BLOB` / メタデータ | バイナリ肥大化を防ぐため、デフォルトはサイズ・種別サマリまたは除外（明示選択時のみ `BLOB`）。 |
| GUID / ReplicationID | `Text` | `TEXT` | 標準 UUID 文字列 (`xxxxxxxx-xxxx-...`) へ正規化。 |

---

## 4. Turso 連携とキャッシュ／マテリアライズ戦略

既存の `SqliteQuerySession::from_disk_datasets` は毎回ソースをスキャンして一時 Turso DB を構築する。しかし `.accdb`（数万〜数十万行）や `.ost` / `.edb`（数百 MB〜数 GB）をエージェントが探索フェーズで何度もクエリする場合、毎回の全走査はレイテンシとメモリ予算（`DATASET_MEMORY_LIMIT` / `MAX_CELLS = 5_000_000`）のボトルネックになる。

これを解決するため、以下の 2 段階アクセスを提供する。

1. **オンデマンド・テーブル単位ステージング（既存 `datasets` 拡張）**
   - `datasets` 配列で `{"source": "legacy.accdb", "table": "受注明細", "alias": "orders", "columns": [...]}` のように指定されたテーブルのみを `jetdb` 等で読み出し、Turso へロードする。
   - CSV / Parquet / SQLite とそのまま SQL `JOIN` 可能。
2. **Turso マテリアライズ／ワークスペースキャッシュ（`jet_export_sqlite` / 自動キャッシュ）**
   - 対象 DB（または選択テーブル群）をローカルの Turso 互換 `.sqlite` ファイルへ一括変換・永続化する（CLI / MCP / UI）。
   - 変換後は既存の `sqlite_list_tables`, `sqlite_describe_table`, `sqlite_count_rows`, `sqlite_query` が**インデックス付きで高速（ミリ秒オーダー）**に動作する。
   - `.ost` / `.pst` のメール本文等に対しては、Turso 側の FTS5（全文検索）やメタデータテーブル付与の拡張余地も生まれる。

---

## 5. Google Workspace 移行（資産継承）ワークフロー

Coding Agent およびデスクトップユーザーが Access 資産を Google Workspace へ移行する際の標準フローを定義する。

1. **資産棚卸し（Discovery）**:
   - エージェントが `jet_inspect_schema` を呼び出し、テーブル一覧、主キー／外部キー、行数見積もり、保存済みクエリ（名前・SQL テキスト・パラメータ）、フォーム／レポート一覧を取得する。
2. **設計資産の可視化（Documentation）**:
   - リレーションシップ情報から Mermaid `erDiagram` を生成し、既存の Mermaid レンダリング・Markdown ワークスペース（`SoraneWorkspace`）で人間がレビュー可能な移行設計書を出力する。
3. **データ移行（Data Export）**:
   - `sqlite_query_to_table` またはバッチエクスポートにより、正規化済みテーブルを `.xlsx`（Google Sheets へそのままインポート可能）または `.parquet` / `.csv` / `.sqlite` へ書き出す。
   - 既存の機密検査・機械判読性チェック（`xlsx_check_machine_readability`）もそのまま適用可能。
4. **ロジック移行支援（Query / AppSheet / Apps Script）**:
   - 抽出した Access 保存済みクエリ（JET SQL 方言：`IIf`, `Nz`, `#date#`, `*` ワイルドカード等）を、エージェントが Turso SQL 互換または Google Sheets 数式 / BigQuery SQL へ変換し、移行前後の突合検証を Turso 上で実行する。

---

## 6. MCP ツールおよびセキュリティ設計

[`crates/ibis-doc-service/src/tools_sqlite.rs`](file:///home/antigravity/sorane-dev/docs/crates/ibis-doc-service/src/tools_sqlite.rs) および `ibis-mcp` に以下を追加・拡張する。

### 6.1 既存ツールの拡張
- `DATASET_EXTENSIONS` に `mdb`, `accdb`（Phase 1）、`edb`, `ost`, `pst`（Phase 2）を追加。
- `sqlite_describe_datasets` / `sqlite_query` / `sqlite_query_to_table` / `sqlite_refresh_query_reports` の `datasets` エントリで、SQLite と同様に `table` パラメータを受け付けるようにする（`ost`/`pst` は `messages`, `folders` 等の仮想テーブル名を指定）。

### 6.2 新規読み取り専用ツール（`Access::Read`）
- **`jet_inspect_schema`**: Access ファイルのテーブル、カラム、リレーションシップ、保存済みクエリ定義を構造化 JSON で返す（行データは返さない）。
- **`ese_list_tables`** / **`ese_describe_table`**: ESE/JET Blue ファイルのカタログ情報を返す。
- **`pst_list_folders`**: `.ost`/`.pst` のフォルダ階層とアイテム数を返す。
- **`jet_export_sqlite`**（任意、または CLI サブコマンド優先）: `--allow-dir` 配下の安全な出力先へ Turso 互換 `.sqlite` を生成する。

### 6.3 セキュリティ・リソース境界
- **ディレクトリ制限**: 既存の `paths` モジュール（`--allow-dir` による実パス検証・シンボリックリンク解決）を厳格に適用する。
- **読み取り専用・無改変**: `jetdb`, `ese-core`, `outlook-pst` すべて読み取り専用で開き、元ファイルを一切変更しない。ロック中・破損・ダーティシャットダウン状態のファイルは明確なエラーメッセージを返す。
- **パスワード／暗号化ファイル**: Access のページ暗号化（RC4/AES）やパスワード保護 DB、暗号化 OST については、未対応または明示エラーとして安全に失敗させ、クラッシュやハングを防ぐ。
- **予算制限**: `MAX_ROWS`, `MAX_CELLS`, `MAX_TEXT_BYTES`, 3 秒のクエリ実行予算、およびレスポンスサイズ上限を既存パイプラインと共有する。

---

## 7. 段階的実装ロードマップ

### Phase 1: Microsoft Access（`.mdb` / `.accdb`）× Turso × MCP 統合
1. `Cargo.toml` に `jetdb` を追加し、`cargo deny check licenses` が通ることを確認する。
2. `crates/ibis-doc-model/src/query/jet_source.rs` を実装し、`DiskDataset` および `SqliteQuerySession` と接続する。
3. `crates/ibis-doc-service/src/tools_sqlite.rs`（または `tools_jet.rs`）で `DATASET_EXTENSIONS` 拡張と `jet_inspect_schema` を実装する。
4. 合成フィクスチャ（Apache-2.0 / MIT 互換のテスト用 `.mdb`/`.accdb`、または自前生成）を用いた単体・統合テストを追加する。
5. [`docs/mcp.md`](file:///home/antigravity/sorane-dev/docs/docs/mcp.md) と `.agents/skills/ibis-office/SKILL.md` を更新し、Coding Agent 向けの利用手順を整備する。

### Phase 2: JET Blue（`ese-core`）＆ Outlook（`outlook-pst`）対応 + Turso キャッシュ
1. `ese-core` と `outlook-pst` を追加し、`ese_source.rs` / `pst_source.rs` を実装。
2. 大規模 DB 向けの Turso `.sqlite` マテリアライズ／キャッシュ機構を導入。
3. MCP ツール（`ese_list_tables`, `pst_list_folders`）と結合テストを追加。

### Phase 3: Google Workspace 移行支援の高度化
1. Access リレーションシップ → Mermaid ER 図 + Markdown 仕様書のワンステップ生成。
2. JET SQL → SQLite/Turso SQL の構文差異チェック・変換補助。
3. 必要に応じた `datacraft-jet` 評価（Jet4/ACE 書き戻し・移行過渡期の双方向同期）。
