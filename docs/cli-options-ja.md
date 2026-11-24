<p align="right"><a href="cli-options.md">English</a> | <b>日本語</b></p>

# ibisOffice コマンドラインオプション

ibisOffice は、グラフィカルなオフィススイートとして通常通り起動できるだけでなく、ターミナルやバッチスクリプト、CI/CD パイプラインから利用可能な強力なコマンドラインインターフェース（CLI）を備えています。

画面を表示せずに Word・Excel・PowerPoint ドキュメントを PDF や PNG 画像に高速変換する**ヘッドレス・エクスポート**や、印刷オプションの指定、起動時間のベンチマーク計測などが可能です。

---

## 目次

1. [基本構文](#基本構文)
2. [ファイルを開く・表示オプション](#ファイルを開く表示オプション)
3. [ヘッドレス・エクスポート（--export）](#ヘッドレスエクスポート--export)
4. [印刷・エクスポート詳細オプション](#印刷エクスポート詳細オプション)
   - [Word (.docx) 向けオプション](#word-docx-向けオプション)
   - [PowerPoint (.pptx) 向けオプション](#powerpoint-pptx-向けオプション)
5. [UI スナップショット（--export ui）](#ui-スナップショット--export-ui)
6. [パフォーマンス計測・ベンチマーク](#パフォーマンス計測ベンチマーク)
7. [実用例（レシピ）](#実用例レシピ)

---

## 基本構文

```bash
# 通常の GUI 起動（ファイル指定またはスタートウィンドウ）
ibisOffice [OPTIONS] [FILE]

# ヘッドレス・エクスポート（画面を出さずにファイルを出力して終了）
ibisOffice --export <format> [EXPORT_OPTIONS] <FILE>

# UI スナップショット（画面を出さずにエディタUIのスナップショットを保存）
ibisOffice --export ui [--out-dir <dir>] [--steps <steps>] [<FILE>]

# 起動時間・パフォーマンス測定
ibisOffice [--timing] [--quit-after-first-frame] [<FILE>]
```

### 対応ファイル形式
- **Word 文書**: `.docx`
- **Excel ワークブック**: `.xlsx`
- **PowerPoint プレゼンテーション**: `.pptx`

※ `FILE` を省略して起動した場合は、テンプレート選択や新規作成が可能な「スタートウィンドウ」が開きます。

---

## ファイルを開く・表示オプション

| オプション | 引数 | 説明 |
|---|---|---|
| `FILE` | ファイルパス | 起動時に開く `.docx`, `.xlsx`, `.pptx` ファイルのパス。 |
| `--slide` | `<number>` | 起動時に指定したスライド番号（1 始まり）を直接表示します（PowerPoint 向け）。 |
| `--read-only` | （なし） | 閲覧だけのために開きます。編集はできず、サイドパネルは閉じたままです。 |

**例:**
```bash
# PowerPoint ファイルを開き、最初から 5 枚目のスライドを表示する
ibisOffice --slide 5 presentation.pptx

# 文書を閲覧だけのために開く
ibisOffice --read-only report.docx
```

---

## ヘッドレス・エクスポート（--export）

文書の AI 向け書き出し、引用照合、DOCX の編集プレビューと別名保存は、`ai-json`、`ai-locate`、`ai-preview`、`ai-apply` を使用します。`ai-locate`、`ai-preview`、`ai-apply` には `--ai-query <file>` を、`ai-apply` には新しい出力先として `--out-file <file.docx>` を指定します。[Markdown リファレンスの CLI 実演](markdown-reference-ja.md#cli-で一括編集を試す)

`--require-preview` は `--export ai-apply` 専用です。編集案の `previewSha256` がプレビュー時に返された値と一致する場合に保存できます。

`--ai-receipt` を `ai-apply` に付けると、保存後に出力先と SHA-256 を含む JSON を標準出力に返します。

`ibisOffice --export md --include-metafile-previews --out-dir ./out input.docx` は、本文に元の EMF・WMF を残し、変換できた図の SVG と抽出できた文字を末尾に併記します。文字の順序は SVG の描画順です。輪郭や画像になった文字には OCR が必要です。ibisOffice は Markdown を DOCX に戻すときにプレビューの付録を除きます。

本文にはテキストボックスなどの図形を含めません（含めると DOCX に戻すときに本文の段落になるため）。図の横のテキストボックスに置いた出典などの文字も読むには `--include-shape-text` を付けます。各図形の段落を文書の順（本文、続いてヘッダー・フッター）に末尾の付録に並べます。この付録も DOCX に戻すときに除きます。`--include-metafile-previews` と併用できます。

`--include-xlsx-validations` を XLSX の `--export md` に付けると、表示シートの入力規則を Markdown に含めます。`--export xlsx` で規則を復元できます。[Markdown リファレンスの XLSX 往復](markdown-reference-ja.md#対応状況と残るギャップ)

`--include-xlsx-hidden-axes` を XLSX の `--export md` に付けると、表示シートの非表示行・列の値と式を Markdown に含めます。`--export xlsx` でセルと非表示状態を復元できます。

`--include-xlsx-hidden-sheets` を XLSX の `--export md` に付けると、非表示ワークシートを Markdown に含めます。`--export xlsx` でシートの順序、内容、`hidden` と `veryHidden` の状態を復元できます。入力規則や非表示行・列のオプションと併用できます。

`--include-xlsx-defined-names` を XLSX の `--export md` に付けると、ブック全体とシート単位の定義名を Markdown に含めます。`--export xlsx` で定義式とスコープを復元できます。

ウィンドウを一切表示せず、GPU / CPU レンダリングによりバックグラウンドで高速に出力を行い、完了後に即座に終了します。サーバー環境や自動化スクリプトに最適です。

| オプション | 引数 | 説明 |
|---|---|---|
| `--export` | `<format>` | エクスポート形式を指定します（必須）。<br>・`pdf`: ベクターグラフィックスと埋め込みフォントを含む高品質 PDF<br>・`png`: 各ページ/スライドを PNG 画像として連番出力<br>・`pdf-image`: ページを画像化して埋め込んだ PDF<br>・`ui`: エディタの UI 画面のスナップショット |
| `--out-dir` | `<dir>` | 出力先ディレクトリを指定します（デフォルト: カレントディレクトリ `.`）。ディレクトリが存在しない場合は自動作成されます。 |
| `--width` | `<px>` | PNG エクスポート時の画像の横幅ピクセル数を指定します（デフォルト: `1920`）。アスペクト比は自動維持されます。 |
| `--range` | `<range>` | 出力するスライドまたはページの範囲を指定します（例: `1`, `1,3-5`, `2-`）。省略時は全ページ/全スライドが出力されます。以前の名前 `--slides` も使えますが、警告が出ます。 |
| `--include-layout` | （なし） | スライドのレイアウト情報を付加してエクスポートします。以前の名前 `--layout` も使えますが、警告が出ます。 |

**例:**
```bash
# プレゼンテーション全体を PDF に変換
ibisOffice --export pdf presentation.pptx

# 1ページ目から3ページ目までを幅 2560px の高解像度 PNG として dist/ に保存
ibisOffice --export png --out-dir dist/ --width 2560 --range 1-3 document.docx
```

### JSON・JSONL の構造を閲覧する

デスクトップ版の「ファイル → 構造化データを開く…」から JSON、JSONL、NDJSON を読み取り専用で開けます。「フォルダーを開く」の一覧からも選べます。JSONL は元の行番号付きで表示され、文字列を検索して行を絞り込めます。選んだ行のオブジェクトや配列を展開し、項目を選ぶと元データ上の位置と値を確認できます。整形式の行は、不正な行が混在していても閲覧できます。

単一の JSON では、展開して読み込んだ項目を検索できます。名前がある配列要素には名前が表示されるため、CycloneDX の構成部品や暗号資産を番号と名前で区別できます。配列・オブジェクトの直下は256件ずつ表示し、入力は128 MiB、JSONL は先頭100万行まで閲覧できます。文書エディターの「表示 → 見出しアウトライン」も同じ階層表示を使い、見出しを選ぶと文書内の位置に移動します。

CycloneDX JSON では、対象システムと構成部品・暗号資産・サービス・依存関係の項目数を最初に表示します。構成部品数には対象コンポーネントと暗号資産を含みます。依存関係の `dependsOn` や暗号資産の `signatureAlgorithmRef` などの値を選ぶと、同じファイル内で一意に解決できる `bom-ref` へ移動できます。参照先から文書全体へ戻れます。項目の詳細には、ファイル内で一意に解決できる直接の依存先・依存元・提供先・提供元も表示します。関係する項目名を選ぶと、その詳細へ移動できます。

自治体連携ファイルなどの UTF-8 CSV（BOMあり・なし）も「構造化データを開く…」から選べます。先頭レコードを列名とし、選んだレコードを列番号付きで縦に表示します。同じ名前の列も列番号で区別できます。`項目_1`・`項目_2` のように連番付きの列が複数ある場合は一つのグループにまとめます。コードの先頭ゼロや長い整数、引用符内の改行を保持し、数式を評価しません。項目を選ぶと引用符を含む元の値とバイト位置を確認できます。同じフォルダーに拡張子を `.hed` に置き換えた格納完了通知ファイルがあれば、ファイル名・バイト数・ヘッダーを除くデータ件数を照合します。不一致は通知の値と実際の値を表示します。

Markdown の見出しをアウトラインで選び、下部の「本文のセクションを折りたたむ」を押すと、文書ビューでその見出しの配下を折りたためます。「本文のセクションを展開する」で元に戻せます。折りたたみは表示状態で、Markdown ファイルには保存しません。

### SBOM と脆弱性スキャンの比較（`sbom-review`）

```bash
ibisOffice --export sbom-review \
  --old-sbom old.cdx.json --new-sbom new.cdx.json \
  --old-scan old.osv.json --new-scan new.osv.json \
  --out-file remediation.md
```

各リリースに対応する CycloneDX JSON と OSV-Scanner の `--format json` 出力を渡します。Markdown には旧版・新版の検出バージョン、SBOM に記載されたバージョン、勧告へのリンク、入力ファイルの SHA-256 が入ります。`--out-file` を省くと標準出力に書き出します。入力は各 32 MiB までです。

検出はパッケージと OSV ID の完全一致で比較します。SBOM 側は crates.io、npm、PyPI、Go の Package URL で照合します。新版で検出されず、かつ新版 SBOM の該当パッケージが単一バージョンで旧スキャンの勧告にある `fixed` と一致する場合は、両方の事実を表示します。比較には同一条件で採取したスキャン結果を使用し、スキャン範囲と本番への反映は別途確認してください。

比較結果を表計算で開くには、出力名を `remediation.xlsx` にします。検出結果、旧版・新版の部品一覧、入力の SHA-256、判定の注意事項が別シートになります。部品一覧は SBOM に記載された構成を表示します。

比較をせずに単体の SBOM やスキャン結果を表計算で開く場合は、次のように変換します。

```bash
ibisOffice --export sbom-table bom.cdx.json --out-file components.xlsx
ibisOffice --export sbom-table scan.osv.json --out-file findings.xlsx
```

デスクトップ版では **ファイル → 開く** から対応する JSON を開くと、編集可能な未保存のブックになります。`Findings` に担当・期限・判断・理由・証拠 URL を記入し、XLSX として保存できます。

**ファイル → 脆弱性レビューを更新…** で新しい OSV-Scanner JSON を選ぶと、現在の判断欄を引き継いだ新規ブックが開きます。新規検出、バージョン変更、再検出を要確認として表示します。未検出の行は最後に検出されたバージョンとともに残ります。比較元の SBOM や追加のワークシートは元のブックで参照できます。

**ファイル → 脆弱性レビューの要対応を表示…** で、その日の担当未設定、対応期限超過、延期期限切れ、スキャン差分の要確認を新しいブックに一覧表示します。期限はブックの日付セルまたは `YYYY-MM-DD` で入力します。判断を `延期`、`リスク受容`、`Deferred`、`Risk accepted` とした行では、期限を例外の見直し日として扱います。当日は期限内です。対応状況の記入は元のブックで行ってください。

CycloneDX JSON に暗号資産が含まれる場合は、暗号資産とその参照関係を別シートに表示します。暗号関連資材の値は表に転記されません。[CBOMの合成サンプル](../test/fixtures/cbom/README.md)

旧版・新版の暗号資産と参照関係の差分をXLSXで確認するには、`--export cbom-compare --old-sbom before.cdx.json --new-sbom after.cdx.json --out-file changes.xlsx` を使用します。暗号資産は `bom-ref` で照合します。

---

## 印刷・エクスポート詳細オプション

Microsoft Office の印刷・エクスポート詳細設定に準拠した出力制御が可能です。

### Word (.docx) 向けオプション

| オプション | 説明 |
|---|---|
| `--print-backgrounds` | 背景の色とイメージ（ページ背景）を出力に含めます（Word の「背景の色とイメージを印刷する」オプション準拠）。 |
| `--print-hidden-text` | 隠し文字を出力に含めます（Word の「隠し文字を印刷する」オプション準拠）。 |
| `--no-print-markup` | 変更履歴やコメント（校閲マークアップ）を出力から除外します（Word の「変更履歴とコメントを印刷する」をオフにする設定準拠）。 |

**例:**
```bash
# 背景画像を含め、校閲マークアップを除外してクリーンな PDF を生成
ibisOffice --export pdf --print-backgrounds --no-print-markup report.docx
```

### PowerPoint (.pptx) 向けオプション

#### カラーモード
| オプション | 引数 | 説明 |
|---|---|---|
| `--print-color` | `color` \| `gray` \| `bw` | 印刷カラーモードを指定します（デフォルト: `color`）。<br>・`color`: カラー印刷<br>・`gray`: グレースケール印刷<br>・`bw`: 単純白黒（ハイコントラスト白黒印刷） |

#### 配布資料（ハンドアウト）設定
| オプション | 引数 | 説明 |
|---|---|---|
| `--handout` | `<n>` | 1ページに複数のスライドを配置する配布資料形式で出力します。<br>指定可能な値: `1`, `2`, `3`, `4`, `6`, `8`, `9`, `16` |
| `--handout-landscape` | （なし） | 配布資料の用紙の向きを「横向き（ランドスケープ）」にします。 |
| `--handout-order` | `horizontal` \| `vertical` | 配布資料のスライドの並び順（デフォルト: `horizontal`。PowerPoint と同じ）。`horizontal` は横に並べてから次の行へ、`vertical` は縦に並べてから次の列へ進みます（1 ページ 4・6・9 枚の場合）。 |
| `--print-frame` | （なし） | 各スライドの外周に細い枠線を付けて出力します（「スライドに枠を付けて印刷」）。 |
| `--notes-pages` | （なし） | スライド画像の下に発表者ノートを配置した「ノート ページ」形式で出力します。 |
| `--outline-pages` | （なし） | スライド内のテキスト階層のみを箇条書きで並べた「アウトライン」形式で出力します。 |

#### 配布資料ヘッダー・フッター
| オプション | 引数 | 説明 |
|---|---|---|
| `--handout-header` | `<text>` | 配布資料のヘッダー文字列を指定します。 |
| `--handout-date` | `<text>` | 配布資料の日付欄の文字列を指定します（デフォルト: 今日の日付。配布資料マスターのヘッダーとフッターの設定で日付がオフなら出しません）。 |
| `--handout-footer` | `<text>` | 配布資料のフッター文字列を指定します。 |
| `--handout-number` | （なし） | 配布資料にページ番号を印刷します（デフォルトでオン。配布資料マスターのヘッダーとフッターの設定でオフなら出しませんが、このオプションを付けると出します）。 |
| `--no-handout-date` | （なし） | 配布資料・ノート・アウトラインに日付を印刷しません。 |
| `--no-handout-number` | （なし） | 配布資料・ノート・アウトラインにページ番号を印刷しません。 |

**例:**
```bash
# 1ページに3スライド配置し、枠線付き・横向き・ヘッダー・日付・ページ番号入りで PDF 出力
ibisOffice --export pdf --handout 3 --handout-landscape --print-frame \
  --handout-header "社内勉強会資料" --handout-date "2026-10-16" --handout-number \
  presentation.pptx

# 発表者ノート付き PDF を生成
ibisOffice --export pdf --notes-pages presentation.pptx
```

---

## UI スナップショット（--export ui）

ヘッドレス環境で ibisOffice のエディタ UI 自体の描画状態をスナップショット（PNG）として記録します。UI のレイアウト検証や自動化テストで使用されます。

| オプション | 引数 | 説明 |
|---|---|---|
| `--steps` | `<steps>` | 実行する UI アクションをカンマ区切りで指定します（デフォルト: `start`）。<br>例: `start,pagedown,end` |

**例:**
```bash
# スタートウィンドウの UI 画面を output/ にキャプチャ
ibisOffice --export ui --out-dir output/

# ドキュメントを開いた状態のエディタ画面をキャプチャ
ibisOffice --export ui --out-dir output/ presentation.pptx
```

---

## パフォーマンス計測・ベンチマーク

ibisOffice の高速な起動処理・レンダリング各ステージの所要時間を精密に測定するためのオプションです。

| オプション | 説明 |
|---|---|
| `--timing` | 起動各ステージ到達時のミリ秒（プロセス開始からの経過時間）を標準エラー出力（stderr）に `ibisOffice timing: <stage> <ms>` 形式で出力します。 |
| `--quit-after-first-frame` | ドキュメントおよびウィンドウの初回描画が完了（第2フレーム安定描画後）した時点で、自動的にプロセスを正常終了します。 |

### 計測されるステージ一覧
- `main`: `main()` 関数の実行開始
- `app`: アプリケーションループ・プラットフォーム層初期化完了
- `window`: ウィンドウ生成完了
- `first-frame`: ウィンドウの初回フレーム描画完了
- `second-frame`: 第2フレーム描画完了
- `read`: ファイルデータのディスク読み込み完了（非同期スレッド）
- `parsed`: OOXML（ドキュメントモデル）解析完了
- `loaded`: レイアウト計算完了・ウィンドウへのドキュメント読み込み完了
- `document-frame`: ドキュメント内容の初回描画完了
- `document-second-frame`: ドキュメント内容の第2フレーム描画完了
- `slide-pictures`: 表示中スライドの画像デコード完了
- `all-pictures`: ドキュメント内全画像デコード・サムネイル生成完了

**例:**
```bash
# 起動時間を計測し、描画完了後に自動終了する
ibisOffice --timing --quit-after-first-frame presentation.pptx
```

出力例:
```text
ibisOffice timing: main 0.12
ibisOffice timing: app 12.45
ibisOffice timing: window 28.30
ibisOffice timing: read 31.10
ibisOffice timing: parsed 38.50
ibisOffice timing: loaded 45.20
ibisOffice timing: first-frame 48.90
ibisOffice timing: document-frame 52.10
ibisOffice timing: document-second-frame 68.40
```

---

## 実用例（レシピ）

### 1. ドキュメントの一括 PDF 変換（CI/CD・バッチ処理）
```bash
for file in docs/*.docx slides/*.pptx; do
  ibisOffice --export pdf --out-dir dist/ "$file"
done
```

### 2. Web 掲載用の高解像度サムネイル画像の自動生成
```bash
# スライド 1 枚目を幅 1920px の PNG として書き出し
ibisOffice --export png --range 1 --width 1920 --out-dir thumbnails/ presentation.pptx
```

### 3. 会議用 2 in 1 白黒ハンドアウト PDF の作成
```bash
ibisOffice --export pdf --handout 2 --print-color bw --print-frame \
  --handout-header "2026年10月度 経営戦略会議" \
  --handout-footer "アイビス株式会社 機密" \
  --handout-number --out-dir handouts/ strategy.pptx
```

## 固定レイアウト文書を Markdown に書き出す

```sh
ibisOffice --export md --out-dir ./out input.pdf
```

PDF・XPS（OXPS を含む）・OFD の抽出可能な本文をページ順に書き出します。パスワードなしで開ける暗号化 PDF（所有者が印刷・複写だけを制限したもの。Standard セキュリティハンドラの RC4・AES-128・AES-256）は復号して読みます。利用者パスワードが必要な PDF はその旨を報告します。行の順序は文字の位置から推定します。ビューアで復号できた図形・画像は、ページ上の配置を持つ SVG として本文に添えます。画像だけのページは OCR を行いません。段落や表の意味構造、署名、原本と同じ組版の復元には別の処理が必要です。DOCX・PPTX の EMF/WMF は Markdown 内に元の画像データを保持します。一般のブラウザーでの EMF/WMF 表示は閲覧環境に依存します。

表の列位置を保つには `--text-layout physical` を付けます。各ページの本文を ```` ```text ```` ブロックに入れ、文字を紙面上の位置に応じた桁に置きます（半角 1 桁・全角 2 桁、`pdftotext -layout` 相当）。Markdown のエスケープはしません。既定の `--text-layout reading` は、各行を語の間を空白 1 つで区切った Markdown として書き出します。

```sh
ibisOffice --export md --text-layout physical --out-dir ./out input.pdf
```

本文だけが要るときは `--export txt` を使います。各ページの行をそのまま書き、ページの終わりに改ページ文字（form feed、`pdftotext` と同じ）を置きます。Markdown のエスケープも図形も付けません。描画や画像の展開をしないので `--export md` より速く終わります。`--text-layout physical` も使えます。

```sh
ibisOffice --export txt --text-layout physical --out-dir ./out input.pdf
```

DOCX の浮動画像は、画像データと表示寸法に加え、位置の基準・オフセット・回り込みを Markdown の `docx-anchor` 属性に保持します。この属性を残して DOCX に戻すと浮動画像になります。属性なしの画像はインラインとして読み込みます。

DOCX の画像のトリミングは `docx-crop` に左・上・右・下の順で 10 万分率を、回転と反転は `docx-transform` に角度（度の 60,000 分の 1）・左右反転・上下反転の順で保持します。

DOCX の画像は、代替テキストに含まれる括弧や改行も復元できます。代替テキストのタイトルと説明、装飾画像の指定も保持します。

DOCX の画像補正では、明るさとコントラストを `docx-lum` に10万分率で、二色化の暗色と明色を `docx-duotone` に sRGB の16進値で保持します。

DOCX の画像では、透明にする sRGB 色を `docx-clear` に、画像全体の不透明度を `docx-opacity` に10万分率で保持します。

彩度と白黒二値化のしきい値は、10万分率でそれぞれ `docx-saturation`（0〜400000）と `docx-bilevel`（0〜100000）に保持します。グレースケールは `docx-grayscale=true` で指定します。
