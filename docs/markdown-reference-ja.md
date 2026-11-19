---
title: "Docs Markdown Reference Manual"
---

# Docs Markdown Reference Manual

このファイルは、日本語の Markdown リファレンスと、Docs で表示・保存・再読込を試せるデモを兼ねています。書式や構文を確認する場合は、使用するビルドと同じリビジョンのリファレンスを開いてください。

## 使い方

1. このファイルを Docs で開きます。
2. 表、数式、図とグラフを確認し、コピーを DOCX に保存します。
3. DOCX を開き直して Markdown に書き出し、数式や Mermaid のソースがどう戻るかを確認します。
   - PPTX の場合は CLI で変換し、スライドとして開きます。
   - Windows と macOS では利用するフォントによって改行位置が変わることがあります。

> [!NOTE]
> この原本は検証の入力です。往復で生成されたファイルは別の場所に保存します。

## Markdown の書式

**太字**、*斜体*、~~取り消し線~~、インラインコード `--export md`、[CLI の使い方](cli-options-ja.md)を同じ段落で確認できます。

- 箇条書きの親項目
  - 日本語の子項目
- [x] Markdown を開く
- [ ] 保存後の図を確認する

| 入力 | 主な用途 | 例数 |
| :--- | :---: | ---: |
| Markdown | 文書・スライド | 3 |
| Mermaid | 図・グラフ | 4 |
| Lawtext | 条文 | 2 |

次の式は数式ブロックの例です。

$$
E = mc^2
$$

インラインの分数 $\frac{a_1}{\sqrt{b}}$ や、別行の行列も表示できます。

$$
\begin{pmatrix} a & b \\ c & d \end{pmatrix}
$$

演算子の上下限の配置は `\limits`（上下）または `\nolimits`（右側）で指定できます。次の式を DOCX に保存し、Markdown に書き戻しても、配置の指定を保持します。DOCX ではネイティブの数式として保存されます。

$$
\int\limits_{0}^{1} x^2\,dx = \frac{1}{3}
$$

$$
\sum\nolimits_{i=1}^{n} i = \frac{n(n+1)}{2}
$$

次はコードブロックです。コマンドを実行する場合は、リポジトリのルートで `ibisOffice` を利用できるようにしてください。

```sh
ibisOffice --export docx --out-dir /tmp/docs-reference/docx docs/markdown-reference-ja.md
ibisOffice --export pptx --out-dir /tmp/docs-reference/pptx docs/markdown-reference-ja.md
ibisOffice --export md --out-dir /tmp/docs-reference/markdown /tmp/docs-reference/docx/markdown-reference-ja.docx
```

## Office Open Markdown の往復確認

Office Open Markdown は、Office 文書の構造を人と AI が編集できる Markdown に移し、対応する構造を編集可能な OOXML に戻す取り組みです。数式は LaTeX 記法で書き、DOCX では Word の数式形式である OMML として保存します。

次の複数行の式は、DOCX に変換してから Markdown に戻すと、各行と `&` の整列位置を保ちます。

$$
\begin{aligned}
x &= 1 \\
yy &= 2
\end{aligned}
$$

行列の行・列と、複数行の式の整列位置は、OMML を入力にした往復テストでそれぞれ確認しています。OMML の整列用属性は Markdown の `&` に変換され、DOCX に戻すときは等式配列内の整列マーカーとして保存されます。

### 式にリンクする

本文から [エネルギーの式](#eq_energy) にリンクできます。

$$
E=mc^2
$$ {#eq_energy}

閉じる `$$` の後ろの `{#eq_energy}` は DOCX のブックマークになり、本文の `[エネルギーの式](#eq_energy)` は文書内リンクになります。ラベルは文書内で一意にし、英字で始まる40文字以内の英数字とアンダースコアを使います。リンクの表示文字列は指定したまま保持されます。

### 式・図・表に番号を付ける

本文から式 ({{ref:eq_energy_number}}) を参照します。参照は番号の段落より前にも置けます。

{{seq:equation}} {#eq_energy_number align=right}

`{{seq:equation}}` は DOCX の `SEQ Equation`、`{{ref:eq_energy_number}}` はブックマークを指す `REF` フィールドになります。数式にリンクするラベルと、番号を参照するラベルは別々に付けます。番号だけを参照したい場合は、番号フィールド専用の段落にします。

`{{seq:figure}}` と `{{seq:table}}` は図と表の独立した連番です。たとえば `図 {{seq:figure}} {#fig_setup}` を参照すると、ラベル付きの「図 1」になります。番号を追加して Markdown を読み直すと、各連番と参照が再計算されます。

この節の連番とリンク付き参照は、Markdown に書き戻せます。これらのフィールドに未対応の Word スイッチが付いている場合は、表示文字列を書き出します。

### 本文中の文献引用

`{{cite:Smith-2024}}` は Word の `CITATION Smith-2024` フィールドになります。日本語ロケールを指定する場合は `{{cite:Tanaka_2025:1041}}` と書きます。文献タグには半角英数字と `_`、`-`、`.` を使えます。

本文の単一文献タグと任意のロケールを Markdown と DOCX の間で保持します。ページ指定や複数文献など、未対応のスイッチが付いた引用は、Word に保存された表示文字列を書き出します。Word の文献データは、YAML フロントマターの直後（フロントマターがなければ文書の先頭）の `word-bibliography` フェンスに格納します。本文のタグ名は `b:Tag` と一致させます。フェンスは文書メタデータとして読み込み、本文からは除外します。

````markdown
```word-bibliography
<b:Sources xmlns:b="http://schemas.openxmlformats.org/officeDocument/2006/bibliography">
  <b:Source>
    <b:Tag>Tanaka_2025</b:Tag>
    <b:SourceType>Book</b:SourceType>
    <b:Author><b:Author><b:NameList><b:Person><b:Last>田中</b:Last></b:Person></b:NameList></b:Author></b:Author>
    <b:Title>研究の方法</b:Title>
    <b:Year>2025</b:Year>
  </b:Source>
</b:Sources>
```

本文中の引用 {{cite:Tanaka_2025:1041}}。
````

この例の DOCX を Word for Mac で開き、引用を更新すると「[田中, 2025]」になることを確認しました。Word が文献データに追加した参照順も、Markdown を経由して DOCX に保持されます。

### 参考文献一覧

一覧を置く位置に `[[bibliography]]` を独立した行で書きます。日本語ロケールを指定する場合は `[[bibliography:1041]]` と書きます。DOCX では Word の `BIBLIOGRAPHY` フィールドになり、Word でフィールドを更新すると、文献データから一覧が生成されます。

独立した一覧を Markdown に戻すと、Word が生成した一覧は元の位置のマーカーに戻ります。Markdown 本文で一覧の見出しを自由に指定できます。文献の絞り込みなど、未対応のスイッチを含む一覧は表示テキストとして書き出します。Word for Mac では、文献2件の一覧を生成し、Markdown を経由して作り直した DOCX に一覧フィールドと後続段落が残ることを確認しました。

### 往復の確認範囲と実装順

往復テストでは、Markdown と DOCX の間で変換した後の文書モデルを比較します。数式を例にすると、式の構造、インラインと別行の区別、整列位置、参照先を確認します。記法や XML は変換時に正規化される場合があります。

| 順番 | 対象 | 確認すること |
| :--- | :--- | :--- |
| 1 | 数式への文書内リンク | 合成 DOCX を Word で保存し、Markdown への変換と DOCX への再変換後も数式のブックマークと参照先が一致することを確認済み。 |
| 2 | 式・図・表の番号と参照 | 合成 DOCX で、Word による式・図・表の連番と式番号参照の更新、および Markdown 経由の DOCX への往復を確認済み。 |
| 3 | ページ脚注と文末脚注 | 同じ ID の両注記を区別し、参照・本文・種別の往復を確認済み。 |
| 4 | 引用と参考文献 | 本文の引用タグ・ロケール、Word の文献データ、基本の参考文献一覧フィールドの往復を確認済み。 |
| 5 | 複雑な表 | 横結合・縦結合・入れ子表の往復と原本のセル引用を確認済み。 |

### ファイル形式ごとの往復範囲

| 形式 | Markdown で確認できる内容 | 往復で差が出る例 |
| :--- | :--- | :--- |
| DOCX | 本文、見出し、リスト、リンク、ページ脚注・文末脚注、結合・入れ子表、対応する数式と連番・参照 | 詳細なページ組版、任意の Word フィールド、注記内の表、数式内の文字色 |
| PPTX | スライド区切り、本文、画像、表、グラフ、発表者ノート | 図形の配置やマスター、アニメーション、表の結合 |
| XLSX | 表示対象のセルを表に変換。AI JSON では値の型と数式も確認可能 | Markdown 表から元の数式ブックや非表示部分、入力規則を再構成すること |

## 表のセル結合

### 横方向

結合するセルの末尾に `<!-- colspan:2 -->` を付け、続く列を空セルにします。数字は、結合する先頭のセルを含めた列数です。次の表を DOCX に保存して Markdown に戻すと、列の位置と結合の指定を保持します。

| 集計対象 <!-- colspan:2 --> | | 件数 |
| :--- | :--- | ---: |
| 国内 | 文書 | 12 |
| 海外 | 文書 | 8 |

標準的な Markdown 表示ではコメントが隠れ、続く列には空セルが表示されます。結合範囲に値がある場合、Docs は各セルを個別に読み込みます。

### 縦方向

結合元に `<!-- vmerge:start -->`、直下の空セルに `<!-- vmerge:continue -->` を付けます。3行以上の結合は、対象の各行に継続マーカーを付けます。横方向にも結合する場合は、各行のセルに同じ `colspan` を先に書きます。

| 分類 | 内訳 | 件数 |
| :--- | :--- | ---: |
| 国内 <!-- colspan:2 --> <!-- vmerge:start --> | | 12 |
| <!-- colspan:2 --> <!-- vmerge:continue --> | | 8 |
| 海外 | 文書 | 5 |

DOCX との往復では列幅の範囲と結合の開始・継続を保持します。継続セルに内容がある場合や、直上のセルと開始列・結合列数が合わない場合は、そのセルを縦結合しません。

### 入れ子表

セル内のパイプ表を `<!-- nested-table: … -->` で囲みます。内側の表の行は `<br>` で区切り、内側のパイプには `\|` と書きます。内側の表に含まれる `&`、`<`、`>` はそれぞれ `&amp;`、`&lt;`、`&gt;` にします。

| 分類 | 内訳 |
| :--- | :--- |
| 国内 | 紙と電子<br><!-- nested-table: \| 媒体 \| 件数 \|<br>\| --- \| ---: \|<br>\| 紙 \| 3 \|<br>\| 電子 \| 9 \| --><br>計12件 |

Docs はセルの中に編集可能な表を表示します。DOCX から Markdown に戻すと、内側の表と前後の文章を保持します。Markdown の読み込みは外側の表を除いて最大8階層です。HTML コメントを非表示にする Markdown ビューアーでは、内側の表はソースで確認できます。

## Markdown 構文一覧

ここでの対応は、入力を読み取って Docs の文書モデルに反映できることを指します。保存形式や書き出し先によって保持される情報は異なります。Markdown に戻すと記法や空白が正規化されることがあります。

| 分類 | 入力構文 |
| :--- | :--- |
| 見出し・段落 | `#` から `######`、見出しの `{#anchor align=center}`、段落の `{align=left}` / `center` / `right` / `justify` / `distribute` |
| 引用・通知 | `>`、`[!NOTE]` / `[!TIP]` / `[!IMPORTANT]` / `[!WARNING]` / `[!CAUTION]` |
| リスト | `-` / `*` / `+`、`1.`、入れ子、`[ ]` / `[x]` のタスク項目 |
| コード・数式 | バッククォートまたはチルダのコードフェンス、インラインコード、`$...$`、`$$...$$`、`math` / `latex` / `tex` フェンス |
| 表 | [横・縦結合と入れ子表](#表のセル結合)。パイプ区切り、`:---` / `:---:` / `---:` で列を左・中央・右揃え |
| 区切り | `---` / `***` の水平線、`\newpage` / `::: pagebreak` / `<!-- pagebreak -->`、`\columnbreak` / `::: columnbreak` |
| 目次・脚注 | `[[toc]]` / `::: toc`、`[^id]` と `[^id]: 本文`、文末脚注の `[^endnote-1]` |
| 連番・相互参照 | [式・図・表の番号](#式図表に番号を付ける) の `{{seq:equation}}`、`{{seq:figure}}`、`{{seq:table}}`、`{{ref:名前}}` |
| 強調・文字装飾 | `**太字**`、`*斜体*`、`***太字斜体***`、`~~削除線~~`、`++下線++`、`==強調表示==`、`^上付き^`、`~下付き~` |
| 日本語組版 | `{漢字\|かんじ}` / `｜漢字《かんじ》` のルビ、`[25]{.tcy}` の縦中横、`《《傍点》》` / `[傍点]{.em}` |
| リンク・画像 | `[名前](URL)`、`[名前](#anchor)`、`![代替テキスト](画像)`。画像には相対ファイルパスや Data URI、`{width=... height=...}` の寸法指定を利用できます。 |
| 変更履歴・コメント | `{++挿入++}`、`{--削除--}`、`{~~旧~>新~~}`、`{>>著者: コメント<<}` |
| 文献引用 | [本文中の文献引用](#本文中の文献引用) の `{{cite:タグ}}` と `{{cite:タグ:ロケール}}`、[参考文献一覧](#参考文献一覧) の `[[bibliography]]` と `[[bibliography:ロケール]]` |
| 図・グラフ | `mermaid` と `chart` フェンス。対応する図種・グラフ種は下記を参照してください。 |
| プレゼンテーション | `marp: true` / `slides: true` / `type: presentation` / `type: slides`、スライド区切り、`<!-- notes: 発表者ノート -->`。 |
| スプレッドシート | CLI で複数の Markdown 表を XLSX に変換できます。見出しと表を使う経路で、一般文書のレイアウトとは別に扱います。 |

### 文書の front matter

ファイル先頭の `---` で囲んだ設定を使います。下表の `/` はキーの別名を表します。

| 用途 | キー |
| :--- | :--- |
| 文書情報 | `title`、`subtitle` / `subject`、`author` / `creator`、`keywords` / `tags`、`description` / `summary`、`category`、`company` |
| ページ | `size` / `paper`、`orientation`、`margins`、`columns`、`column_separator`、`direction` / `text_direction`、`line_numbers`。ページ設定には `page.` 付きの別名もあります。 |
| 書体・段落 | `font` / `font_family`、`east_asian_font`、`font_size`、`style_set`、`paragraph_spacing` |
| 文書の表示・編集 | `header`、`footer`（`{page}` と `{filename}`）、`watermark`、`page_color`、`track_changes` |

代表値は次のとおりです。寸法には `mm`、`cm`、`in`、`pt` などの単位付き指定を使えます。

- `orientation` は `portrait` / `landscape`。縦書きは `direction: vertical`（`tbrl`）、横書きは `horizontal`（`lrtb`）。
- `margins` は `normal` / `narrow` / `wide` または寸法。各辺には `margins.top`（`page.margins.top`、`margin_top`）などを指定できます。辺の名前は `top` / `bottom` / `left` / `right` です。
- `columns` は 1〜12、`line_numbers` は `true` / `page` / `continuous`。後者はページをまたいで行番号を振ります。
- `style_set` は `default` / `casual` / `centered` / `formal` / `elegant` / `modern` / `simple` / `minimalist`。
- `paragraph_spacing` は `default` / `no_space`（`no_paragraph_space`、`none`）/ `compact` / `tight` / `open` / `relaxed` / `double`。

### ネイティブグラフの chart フェンス

`type`（`kind`、`chart`）、`title`、`width`、`height`、`categories`（`labels`、`x`）を指定できます。データは CSV 行、または `series:` / `data:` の下に `- 売上: [10, 20]` や `- name: 売上, values: [10, 20]` の形式で書きます。

| 分類 | `type` の値 |
| :--- | :--- |
| 縦棒 | `column` / `clustered-column`、`stacked-column`、`percent-stacked-column` / `100%-stacked-column` |
| 横棒 | `bar` / `clustered-bar`、`stacked-bar`、`percent-stacked-bar` / `100%-stacked-bar` |
| 折れ線・面 | `line`、`stacked-line`、`area`、`stacked-area` |
| 円・その他 | `pie`、`doughnut` / `donut`、`radar`、`scatter` |

次の例では、編集可能なグラフが一つ表示されます。

```chart
type: column
title: 月別の原稿数
categories: [1月, 2月]
series:
- 原稿: [10, 20]
```

## Mermaid

`mermaid` フェンス内の flowchart、sequence、ER、state、Gantt、ZenUML などは SVG として表示されます。`pie` と `xychart` は通常、編集可能な Office グラフになります。取り込み経路によっては SVG になります。グラフを編集すると、再書き出し時には編集結果を表す `chart` ブロックになる場合があります。[Mermaid の対応範囲](mermaid-compatibility.md)

### デモを開く

[図種別デモ](mermaid-demo-ja.md) は、そのまま Docs で開ける Markdown です。代表例を図種ごとに確認できます。このリファレンスでも、日本語ラベル、双方向矢印、横棒グラフ、数値 x 軸を試せます。

公式コーパスの全例を、図種別の Markdown として生成できます。目次には描画結果と、対応・未実装・対象外として追跡する構文の一覧が載ります。各例には、該当する既知の制約を表示します。

```sh
MERMAID_CORPUS_DEMO=/tmp/ibis-mermaid-demo \
  cargo test -p ibis-pptx --test mermaid_corpus
```

[Web 版の構文デモ集](https://docs.sorane.dev/samples/mermaid/web) には、固定した Mermaid 12.3.0 の公式例539件があります。ローカル生成先では `index.md` が目次です。出典と MIT ライセンスは生成先に含まれ、Mermaid CI では `mermaid-markdown-demo` 成果物として取得できます。

### 対応する図種と構文

第1列は現在認識するすべての図種キーワードと別名です。第2列は公式例コーパスで対応として追跡している項目です。設定キーや演算子は[構文一覧の元データ](../crates/ibis-pptx/tests/mermaid/features.txt)と[図種ごとの入力例](../crates/ibis-pptx/tests/mermaid/corpus/)で照合できます。

共通設定は図の先頭の YAML に書きます。タイトル、配色テーマ、主要な色変数を扱います。HTML ラベルは SVG の通常テキストとして表示します。ELK 固有の配置、任意の CSS、コールバック、アニメーションは再現しません。

| 図種キーワードと別名 | コーパスで追跡している対応構文・設定 |
| :--- | :--- |
| `pie` | title, showData, slice values / config: pie options (textPosition, donutHole, highlightSlice, legendPosition) / config: pie theme variables (pie1-12, pieOuterStrokeWidth) |
| `xychart` / `xychart-beta` | title, horizontal orientation, categorical / numeric x-axis, bar / line / y-axis title / range / point labels (line [540 "PaLM", ...]) / config: xyChart options (width, height, showDataLabel, showDataLabelOutsideBar) / config: xyChart theme variables (plotColorPalette, titleColor) |
| `zenuml` | 参加者・別名・主要な注釈、同期・非同期メッセージ、生成、戻り値、ループと分岐、並行処理、try / catch / finally。 |
| `sequenceDiagram` | rect highlight / create / destroy participants / par/and, critical/option dividers / lower-case note / participant types @{ "type" } / bidirectional arrows <<->> |
| `flowchart` / `flowchart-v2` / `flowchart-elk` / `graph` | classDef / class / ::: styling / style statement / linkStyle / bidirectional / circle / cross edges / invisible link ~~~ / expanded node shapes @{ shape } / node icons @{ icon } (drawn as a symbol in a box) / edge ids (e1@-->) / edge curve e1@{ curve } / collapsed subgraph @{ view } / markdown strings "`...`" / Font Awesome icons (fa:) as Unicode symbols |
| `stateDiagram` / `stateDiagram-v2` | note / concurrency -- / classDef / class / ::: |
| `classDiagram` / `classDiagram-v2` | namespace / note / classDef / style / ::: / generics ~T~ / relation cardinality "1" --> "\*" / lollipop interfaces ()-- / config: class.hideEmptyMembersBox / config: class.hierarchicalNamespaces (dotted namespaces nest) |
| `erDiagram` | entity alias p[Person] / word cardinality (one or more, 1 to ...) / subgraph / direction / classDef / style / ::: / markdown in entity names (\*\*bold\*\* _italic_) |
| `gantt` | axisFormat / tickInterval / weekday / weekend / vert markers / todayMarker / until / config: gantt options (useWidth, topAxis, rightPadding, numberSectionStyles, barHeight, barGap) |
| `mindmap` | node shapes ((circle)) {{hexagon}} / icons ::icon() (drawn as a named badge; icon fonts are not embedded) / markdown strings "`...`" / config: colorBy (depth) / config: branch colours (cScale, cScaleLabel) / config: root colour (git0, gitBranchLabel0) |
| `gitGraph` | cherry-pick / commit type (HIGHLIGHT / REVERSE) / branch order / direction (gitGraph LR: / TB: / BT:) / config: gitGraph options / config: git theme variables (git0-7, gitInv, gitBranchLabel, commit / tag labels) |
| `journey` | `title` / `section` / `Task: 5: Actor` |
| `timeline` | section / config: period colours (cScale, cScaleLabel, disableMulticolor) |
| `quadrantChart` | point styling (radius, color, :::) / config: chartWidth / chartHeight / pointRadius / config: quadrant theme variables (quadrantNFill / TextFill, point / axis / title fills) |
| `requirementDiagram` | requirement / element blocks and relationships / reverse relationships (<- kind -) / classDef / class / ::: styling / style statement / direction / markdown names and text ("\*\*bold\*\*", "\*italic\*") |
| `kanban` | card metadata @{ ticket, assigned, priority } / config: section width |
| `packet` / `packet-beta` | relative field widths (+N) / title line / config: packet options (showBits, bitOrder, bitsPerRow, bitWidth, rowHeight, padding) / config: packet theme variables (startByteColor, blockFillColor ...) |
| `sankey` / `sankey-beta` | quoted CSV fields / config: sankey options (showValues, labelStyle, nodeWidth, nodePadding, linkColor, nodeAlignment) / config: node colours (nodeColors) |
| `block` / `block-beta` | columns (N / auto) / nested blocks (block:id:span ... end) / widths (id:N) and space / space:N / block arrows <["label"]>(right, left, up, down, x, y) / edges with labels (A-- "X" -->B) / style / classDef / class |
| `architecture-beta` / `architecture` | groups (group id(icon)[title] in parent) / junctions / edge sides and arrows (a:R --> L:b) / align row / column / built-in icons (cloud, database, disk, internet, server) |
| `C4Context` / `C4Container` / `C4Component` / `C4Dynamic` / `C4Deployment` | title line / boundaries (Enterprise_ / System_ / Container_Boundary, Boundary) / deployment nodes / Db / Queue / _Ext element variants / BiRel / Rel_Back / Rel_U, _D, _L, _R / UpdateElementStyle ($bgColor, $fontColor, $borderColor) / UpdateRelStyle colours ($textColor, $lineColor) / UpdateLayoutConfig ($c4ShapeInRow, $c4BoundaryInRow) / element tags ($tags, no visual effect without AddElementTag) |
| `treemap` / `treemap-beta` | nested sections by indentation / classes (:::class, classDef) / config: diagramPadding / width / height / config: valueFormat ($, thousands, .Nf, .N%) |
| `venn` / `venn-beta` | title line / unions of two or three sets / set and union sizes (:N) / text items in a region / style (fill, stroke, color) |
| `radar` / `radar-beta` | multiple axis / curve lines / max / min / graticule polygon / title line / config: axisScaleFactor, axisLabelFactor, curveTension, width, height, margins / config: curve colours (cScale) / config: radar theme variables (curveOpacity, curveStrokeWidth, axisColor, graticuleColor, graticuleOpacity) |
| `usecase` / `usecase-beta` | actors (normal / hollow / awesome / fa: icon) / use cases (ellipse / rect, business, «stereotype») / system boundaries and packages / direction / links (-->, <--, --, o / x / triangle ends, labels) / include / extend links (..> : include) / edge ids (A id@--> B) and edge styles / notes (note for X) / json boxes / classDef / class / style / :::class / accTitle / accDescr / config: actor / usecase fonts / config: nodeSpacing / rankSpacing / diagramPadding / config: colorScheme (rotate palette; role draws the default colours) / config: useMaxWidth (no effect; the SVG has a fixed size) |
| `wardley` / `wardley-beta` | components and anchors at [visibility, evolution] / component label offsets, (inertia), (build/buy/outsource/market) / dependency links (->) / flows (+>, +<, +<>, +'label'>) / evolve / custom evolution stages (@ widths, dual labels) / size / title / pipelines / notes / annotations legend / accelerators / deaccelerators |
| `treeView-beta` / `treeView` | box-drawing prefixes (├── └── │) / :::class highlight / ## comments / config: showIcons (generic folder / file glyphs) / config: rowIndent / lineThickness / config: treeView theme variables (labelFontSize, labelColor, lineColor) |
| `swimlane-beta` / `swimlane` | lanes (subgraph id [title] … end) / links with labels and chains / flowchart node shapes / classDef / class / style / accTitle / accDescr |
| `eventmodeling` | timeframes (tf / timeframe) / reset frames (rf / resetframe) / inline payloads { … } / data blocks and [[references]] / explicit sources (->> NN) / streams (Stream.Event lanes) |
| `ishikawa-beta` | 最初の行に結果、その後に原因。インデントで下位要因を指定。 |
| `cynefin-beta` | 5領域、引用符付き項目、タイトル、ラベル付き遷移、幅・高さ・余白、説明文の表示、境界の振幅。遷移は図の下に表示。 |
| `agentflow-beta` / `agentflow` | flows (flow id["Agent"] … end, nested) / global block / node shapes (input / task / tool / decision / refdoc / action) / links (-->, -.-, --x, -- text -->) / connectors / collapsed flows (@{ view: collapsed }) / metadata (protocol, endpoint, connectorRef, params, returns, value, instruction; not drawn) / config: nodeSpacing / rankSpacing |
| `railroad-beta` / `railroad-ebnf-beta` / `railroad-abnf-beta` / `railroad-peg-beta` | EBNF rules (alternatives, ? \* +, groups, [ ] and { }) / ABNF rules (/ and n\*m repetition) / PEG rules (<- / ! &) / function form (sequence / choice / optional / zeroOrMore / oneOrMore / terminal / nonterminal) / title |

### 図の描画

箱、矢印、日本語ラベルを確認します。

```mermaid
flowchart LR
  A[Markdown を開く] --> B{図を描画できる?}
  B -->|はい| C[図を表示]
  B -->|いいえ| D[コードを表示]
```

双方向の実線と破線で、参加者が二人のまま両端に矢印が付くことを確認します。

```mermaid
sequenceDiagram
  participant A as 著者
  participant D as Docs
  A<<->>D: 原稿とプレビュー
  D<<-->>A: 保存と再読込
```

### DOCX・PPTX 内で Mermaid ソースを編集

Markdown から取り込んだ SVG の Mermaid 図を DOCX で選び、書式メニューの「Mermaid ソースを編集…」を開きます。上のフローチャートのラベルを変えて「適用」を押すと、図が更新されます。取り消しでは図とソースが一緒に戻ります。保存して開き直し、Markdown に書き出すと更新したソースを確認できます。

描画できないソースを入力するとエラーを表示します。ソース編集は元の Mermaid を保持している DOCX の画像が対象です。

PPTX では Mermaid の SVG 図を一つ選び、図の書式メニューから「Mermaid ソースを編集…」を開きます。新しい図は元の枠内に縦横比を保って収まり、中心位置を保ちます。Office グラフになった図はグラフの編集機能を使用してください。

### Office グラフ

凡例と値を確認します。小さい扇形は SVG フォールバック時のラベル確認にも使えます。

```mermaid
pie showData
  title 原稿の内訳
  "本文" : 72
  "図表" : 25
  "注記" : 3
```

横向きは `xychart horizontal` または `config.xyChart.chartOrientation: horizontal` で指定できます。棒だけの場合は編集可能な横棒グラフになり、折れ線がある場合は SVG で表示します。

```mermaid
xychart horizontal
  title "申請の処理件数"
  x-axis [四月, 五月, 六月]
  y-axis 0 --> 40
  bar [18, 25, 34]
```

数値 x 軸は `x-axis "経過秒" 0 --> 10` のように指定します。系列ごとに範囲の両端を含めて等間隔に点を配置し、1点だけの場合は下限に配置します。数値 x 軸を使う図は SVG として表示し、Markdown に戻すと Mermaid ソースを復元します。

```mermaid
xychart
  x-axis "経過秒" 0 --> 10
  y-axis "処理件数" 0 --> 40
  line "実績" [8, 21 "中間", 35]
  line "目標" [10, 38]
```

### コード表示に戻る場合とフェンスの選び方

未対応の構文や解釈できない図は `mermaid` のコードブロックとして表示します。DOCX・PPTX を経由しても Markdown に書き出すとコードが戻ります。SVG として描画された図でも、ラベルの見え方は実行環境で利用できるフォントによって変わります。

本文にバッククォートやチルダだけの行がある場合は、その行より長いフェンスか、もう一方の記号を使います。Mermaid を書き出すときは、本文と衝突しない長さのフェンスを選びます。

### ZenUML

ZenUML は `mermaid` フェンスの先頭に `zenuml` と書きます。ibisOffice は対応する呼び出し、戻り値、分岐などをシーケンス図の SVG として描画します。次の例では、呼び出し前の `//` コメントが注記になり、呼び出しと戻り値が別々の矢印になります。

```mermaid
zenuml
  @Actor 執筆者
  // 原稿を確認
  執筆者->査読者.確認() {
    return 確認済み
  }
```

DOCX・PPTX では図を選んで「Mermaid ソースを編集…」から修正できます。Markdown に書き戻すと、描画に使ったシーケンス図の内部表現ではなく、保存した ZenUML の本文が戻ります。[ZenUML の図種別デモ](mermaid-demo-ja.md#zenuml)

## 法令標準XML と Lawtext

Docs は UTF-8 の法令標準XMLや、法令名・法令番号と条見出しを持つ Lawtext を認識し、条文を Markdown に変換して開きます。文書全体を法令として扱う挙動は、同梱の Lawtext と XML をそれぞれ開いて試せます。

Lawtext 本家が掲載している行政手続法の第一条を、法令名と法令番号が別行の原文のまま同梱しています。次の例では、章見出し、条の見出し、第二項を確認できます。[Lawtext 本家の掲載例](https://github.com/yamachig/Lawtext/blob/60567c6ca7de6b14c8c1d895338c4614b49969d1/README.md#lawtextの例--example-of-lawtext)・[同梱した入力](../crates/ibis-pptx/tests/lawtext/administrative-procedure-act.lawtext)。

```sh
ibisOffice --export md --out-dir /tmp/docs-reference/law crates/ibis-pptx/tests/lawtext/administrative-procedure-act.lawtext
ibisOffice --export lawxml --out-dir /tmp/docs-reference/law crates/ibis-pptx/tests/lawtext/administrative-procedure-act.lawtext
ibisOffice --export md --out-dir /tmp/docs-reference/law crates/ibis-pptx/tests/lawtext/321CONSTITUTION.xml
```

日本国憲法の XML では、Markdown に前文が出ることと、ibisOffice が生成した Lawtext を読み直すと本則の構造が一致することを CI で確認します。改正履歴や全 XML 要素の可逆性、他の Lawtext 構文との互換性は個別に確認が必要です。[CI の入力と出典](../crates/ibis-pptx/tests/lawtext/README.md)。

### Lawtext の構文と法令標準 XML の対応範囲

| 構造 | Lawtext での表記 |
| :--- | :--- |
| 法令名・番号 | `行政手続法（平成五年法律第八十八号）`、または法令名の次の行に `（平成五年法律第八十八号）` |
| 編・章・節・款・目 | `第一編`、`第一章 総則`、`第一節`、`第一款`、`第一目`。見出しの段を保持します。 |
| 条・枝番・見出し | `第一条 本文`、`第一条の二 本文`。`（目的等）` は条に先立つ行に書きます。 |
| 項 | 条に続く本文は第一項、`２ 本文` や `2 本文` は番号付きの項になります。 |
| 号・号の下位項目 | 項の下で、行頭に空白を2文字以上置いた `一 本文` を号とし、号の下で4文字以上置いた `イ 本文` を下位項目とします。 |
| 附則 | `附則`、`附 則`、`附則（改正法令番号）`。附則内の条や項を扱います。 |
| 別表 | `別表` で始まる見出しとパイプ区切りの表。括弧内に関係条を指定できます。 |

法令標準 XML の取り込みでは、`Law` / `LawNum` / `LawBody` / `LawTitle`、`EnactStatement` / `Preamble`、`MainProvision`、編・章・節・款・目、`Article` / `ArticleCaption` / `ArticleTitle`、`Paragraph` / `Item` / `Subitem1` 以降、`SupplProvision`、`AppdxTable` と表の行・列を読み取ります。XML 内のすべての属性や要素が、Lawtext や Markdown への変換を往復して保持されるわけではありません。

変換した Markdown の先頭には `type: lawtext` と法令情報（`law_title`、`law_num`、`era`、`year`、`num`、`law_type`）が入ります。法令形式で書き出すときは、条見出しや項・号の構造を編集後も保ってください。

## AI 向け JSON 書き出し

DOCX、XLSX、Lawtext、法令標準 XML は CLI から `--export ai-json` で書き出せます。

```sh
ibisOffice --export ai-json --out-dir /tmp/docs-reference report.docx
```

出力は `report.ai.json` です。`markdown` に本文、`sourceSha256` に入力ファイルの SHA-256 が入ります。`markdownBytes` は JSON を読み取った後の Markdown 文字列に対する UTF-8 バイト範囲です。開始位置を含み、終了位置は含みません。

- DOCX の `blocks` は本文の段落・表と、ページ脚注・文末脚注を指します。本文の `index` は 0 始まり、`modelId` は読み込んだ時点の文書モデル内の ID です。注記の `id` は元の DOCX の値です。`part` が `footnotes` か `endnotes` かで、同じ ID の注記を区別できます。
- DOCX の `tables` には本文の表の構造が入ります。`source` は `blocks` 内の表と共通です。`structure.rows` と各行の `cells` は文書順で、`row` と `column` は 1 始まりです。`header` は繰り返し見出し行の指定を表します。
- セルの `columnSpan` は横方向の結合列数です。`verticalMerge` は `none`、`start`、`continue` のいずれかです。セル内の `blocks` には段落の Markdown と入れ子表の構造が、原文と同じ順番で入ります。段落やセルの `modelId` は同じ原本を参照するときに使います。
- DOCX の `tableCells` は、本文のパイプ表に出力されたセル内容の UTF-8 範囲です。`source.tableModelId` と `source.modelId` は表とセルの ID、`row` と `column` は 1 始まりの開始位置です。結合列を数えた原本の列番号が入ります。空セル、結合指定、区切りのパイプにセル範囲は付きません。入れ子表の引用位置は外側のセルまで特定でき、内側の行・列は `tables` に含まれます。
- XLSX の `cells` はシート名と 1 始まりの行列を持ちます。表示文字列、保存された値とその型、数式を別々に記録します。Markdown 出力と同じく表示中のシート・行・列を対象にします。
- Lawtext と法令標準 XML の `articles` は法令番号、条番号、附則の位置と改正法令番号を記録します。本則の附則位置は `null` です。
- `diagnostics` は Markdown 出力で検出した変換上の差を示します。結合セルや入れ子表の構造は `tables` で確認できます。

位置情報は書き出し時の入力に対応します。改版後の照合や、AI の編集案の適用に使う場合は入力の SHA-256 を確認してください。

DOCX のページ脚注と文末脚注に同じ ID がある場合、Markdown は別々のラベルで書き出します。

```markdown
本文[^1]と補足[^endnote-1]。

[^1]: ページ脚注
[^endnote-1]: 文末注記
```

`ai-locate` で注記本文を引用すると、元の DOCX の注記種別と ID が返ります。Markdown を DOCX に戻すと、`endnote-` に正の整数を続けたラベルは、その ID を持つ文末脚注になります。例の `[^1]` はページ脚注、`[^endnote-1]` は文末脚注として保存されます。Markdown を再取り込みすると、注記内の複数行は一つの段落にまとめられます。

### AI の指摘から原文の位置を引く

`--export ai-locate --ai-query citation.json report.docx` は、指定範囲と重なる段落・表・脚注、条文、セルを標準出力に JSON で返します。`citation.json` には `version: 1` に相当する JSON フィールドと、書き出し時の `sourceSha256`、引用した `markdownBytes`、その範囲と完全に一致する `quote` を入れます。入力のハッシュが変わっていた場合や、UTF-8 の文字境界・引用が一致しない場合はエラーになります。

デスクトップ版では `ibisOffice --ai-query citation.json report.docx` で原本を開き、該当段落の先頭へ移動できます。対象は一つの本文段落に対応する引用です。複数段落をまたぐ引用、表や脚注、法令と表計算の位置は `ai-locate` で確認できます。DOCX の表を引用したときは、`matches` に表全体、`tableCellMatches` に引用範囲が重なるセルが入ります。

### 段落の編集案を確認して適用する

引用の JSON に、原文の段落全体のプレーンテキストを `before`、変更後の一行のテキストを `replacement` として追加します。

```sh
ibisOffice --export ai-preview --ai-query proposal.json report.docx
ibisOffice --export ai-apply --ai-query proposal.json --out-file report-reviewed.docx report.docx
```

プレビューは変更前後の段落全体を JSON で返します。適用時にも照合を行い、新規ファイルに保存します。段落全体の置換は一つのテキストランからなる本文段落が対象です。保護・署名・変更履歴の記録が有効な文書や、リンク・ブックマークなどの注釈を含む対象段落は拒否します。

引用箇所だけを置換するには、段落全体のプレーンテキスト `before` 上の範囲を `paragraphBytes` として追加します。どちらの範囲も UTF-8 のバイト単位で、開始を含み、終了を含みません。たとえばエクスポートされた Markdown の先頭が `前**旧**後` の場合、`before` は `前旧後`、`quote` は `旧`、`markdownBytes` は `[5, 8]`、`paragraphBytes` は `[3, 6]` です。`replacement` に `新` を指定すると、太字のまま `旧` が `新` に変わります。両方の範囲が同じ箇所を指し、引用が一つのテキストラン内に収まる必要があります。

### 表のセルの編集案を確認して適用する

`ai-preview` と `ai-apply` を使います。編集案の `target` は `"tableCell"` とし、`markdownBytes` は `tableCells` にあるセル全体の範囲を指定します。`quote` はその範囲の Markdown、`before` はセル全体のプレーンテキストです。たとえば太字のセルでは `quote` が `**東京**`、`before` が `東京` になります。`replacement` に `大阪` を指定すると、保存後も太字のままです。

セルの一部を置換する場合は、`markdownBytes` と `quote` に該当箇所を指定し、`paragraphBytes` に同じ箇所の `before` 上の UTF-8 バイト範囲を追加します。`before` には引き続きセル全体のプレーンテキストを入れます。たとえば `東京と**東京**` の太字の「東京」だけを置換する場合、`before` は `東京と東京`、`paragraphBytes` は `[9, 15]` です。

対象は一つの段落を持つ、本文の結合されていないセルです。全体の置換には単一のテキストランが必要です。部分置換は一つのテキストランに収まる引用を対象とし、他のランの書式とテキストを保持します。プレビューには表・セルの ID と行・列が入り、適用時は原本と照合して別の DOCX に保存します。

### 引用箇所を削除する

段落やセルで引用した箇所だけを削除するには、部分置換と同じように `paragraphBytes` を指定し、`replacement` に空文字列 `""` を入れます。たとえば `東京と**大阪**` から太字の「大阪」を削除する場合、`before` は `東京と大阪`、`paragraphBytes` は `[9, 15]` です。削除と置換は同じバッチに含められます。

### 引用の直前・直後に挿入する

挿入位置の目印となる引用を `markdownBytes` と `paragraphBytes` で指定し、`insert` に `"before"`（直前）または `"after"`（直後）を入れます。`replacement` は挿入する一行のテキストです。引用は残り、挿入したテキストは目印のテキストランの書式を引き継ぎます。たとえば太字の「東京」を目印に `insert: "after"` と `replacement: "支店"` を指定すると、太字の「東京支店」になります。

一括編集では、同じ位置への複数の挿入や、置換・削除の範囲の内側または端への挿入を拒否します。

### 複数の編集案をまとめる

`ai-preview` と `ai-apply` には、同じ DOCX に対する複数の編集案も渡せます。JSON の最上位に `version: 1`、原本の `sourceSha256`、編集案の配列 `edits` を置きます。配列の各要素には単独の編集案と同じ `markdownBytes`、`quote`、`before`、`replacement` などを入れます。`version` と `sourceSha256` は最上位から引き継ぎます。

すべての編集案を同じ原本に照合します。一件でも照合に失敗すると出力ファイルは作成されず、エラーには配列内の何件目かが表示されます。同じ段落やセルでも、原本上の範囲が重ならない部分置換をまとめられます。同じテキストラン内でも指定でき、置換後の文字数が変わっても他の範囲には影響しません。範囲が重複する場合は一括適用を拒否します。プレビューの `edits` は入力順に、各案を単独で原本に当てた場合の変更前後を返します。`targets` は段落・セルごとに全案を反映した変更前後を返します。対象は最初に現れた順で、`editIndices` は対応する案の入力順の番号（1 始まり）です。適用時は全件を反映した一つの新しい DOCX を作ります。

### プレビューで変更位置を確認する

単独プレビューと一括プレビューの各 `edits` 要素は、操作の種類を `operation`（`replace`、`delete`、`insert`）で返します。`markdownBytes` と `quote` は照合した引用です。`affectedParagraphBytes` は、原本の段落またはセルのプレーンテキスト `before` のうち、実際に変わる UTF-8 バイト範囲を指します。挿入の場合は開始と終了が同じ位置です。`replacement` はその範囲に入る文字列です。

### 確認した編集案を指定して保存する

`ai-preview` が返す `previewSha256` を編集案 JSON の最上位にコピーできます。`ai-apply` はこの値を指定した場合、照合済みの変更内容から再計算し、一致しなければ保存しません。一括編集では案の順番も照合します。編集案を変更したら、改めてプレビューして新しい値を取得します。

`ai-apply` に `--require-preview` を付けると、`previewSha256` がない編集案も拒否します。

この値は原本に対する編集内容を照合するためのものです。確認者の本人性や承認を証明する電子署名ではありません。

### 保存結果を JSON に記録する

`ai-apply` に `--ai-receipt` を付けると、DOCX の保存に成功した後で結果を標準出力に返します。結果には `outputPath` と `outputSha256`、原本の `sourceSha256`、編集件数 `editCount` が入ります。`previewSha256` は適用した案のハッシュです。`previewVerified` は、編集案にその値が指定され、一致を確認した場合に `true` になります。確認済みの案だけを保存するには `--require-preview` も付けます。

### 編集案 JSON を検証する

単独編集と一括編集の入力形式は [編集案 JSON Schema v1](schemas/ai-edit-proposal-v1.schema.json) で確認できます。Draft 2020-12 のバリデーターを使うと、項目名の誤り、必須項目の不足、空のバッチ、改行を含む置換などを事前に検出できます。

ファイルの SHA-256、UTF-8 の文字境界、引用と原文の一致、編集範囲の重複は原本を使って確認します。スキーマの検証後も、保存する前に `ai-preview` で原本との照合結果を確認してください。

### CLI で一括編集を試す

macOS または Linux のシェルで、`ibisOffice` と `python3` を使います。合成文書を一時ディレクトリに作り、本文と書式付きの表セルをまとめて変更します。書き出された Markdown を UTF-8 に変換してから位置を求めます。

```sh
demo=$(mktemp -d)
cat > "$demo/original.md" <<'EOF'
出張先を確認する。

| 支店 |
| --- |
| **東京**と*大阪* |
EOF
ibisOffice --export docx --out-dir "$demo" "$demo/original.md"
ibisOffice --export ai-json --out-dir "$demo" "$demo/original.docx"
python3 - "$demo" <<'PY'
import json
import sys
from pathlib import Path

demo = Path(sys.argv[1])
bundle = json.loads((demo / "original.ai.json").read_text(encoding="utf-8"))
markdown = bundle["markdown"].encode("utf-8")

def byte_range(text, quote):
    encoded = quote.encode("utf-8")
    start = text.index(encoded)
    return [start, start + len(encoded)]

def edit(quote, before, replacement, **fields):
    return dict(markdownBytes=byte_range(markdown, quote), quote=quote,
                before=before, replacement=replacement, **fields)

cell = "東京と大阪"
batch = {
    "version": 1,
    "sourceSha256": bundle["sourceSha256"],
    "edits": [
        edit("出張先を確認する。", "出張先を確認する。", "出張先を確定する。"),
        edit("東京", cell, "横浜市", target="tableCell",
             paragraphBytes=byte_range(cell.encode("utf-8"), "東京")),
        edit("大阪", cell, "札幌", target="tableCell",
             paragraphBytes=byte_range(cell.encode("utf-8"), "大阪")),
    ],
}
(demo / "batch.json").write_text(json.dumps(batch, ensure_ascii=False), encoding="utf-8")
PY
ibisOffice --export ai-preview --ai-query "$demo/batch.json" "$demo/original.docx" > "$demo/preview.json"
cat "$demo/preview.json"
```

プレビューを確認したら、次のコマンドで照合用のハッシュを編集案に追加して保存します。

```sh
python3 - "$demo" <<'PY'
import json
import sys
from pathlib import Path

demo = Path(sys.argv[1])
batch = json.loads((demo / "batch.json").read_text(encoding="utf-8"))
preview = json.loads((demo / "preview.json").read_text(encoding="utf-8"))
batch["previewSha256"] = preview["previewSha256"]
(demo / "checked.json").write_text(json.dumps(batch, ensure_ascii=False), encoding="utf-8")
PY
ibisOffice --export ai-apply --require-preview --ai-query "$demo/checked.json" \
  --out-file "$demo/reviewed.docx" "$demo/original.docx"
ibisOffice --export md --out-dir "$demo" "$demo/reviewed.docx"
cat "$demo/reviewed.md"
```

プレビューの `targets` には本文とセルの2件が入り、セルの適用後テキストは「横浜市と札幌」になります。保存後の `reviewed.md` では `**横浜市**と*札幌*` になり、太字と斜体を確認できます。`original.docx` は原本のままです。

デスクトップ CI では macOS と Linux のビルド済みバイナリに対して、同じ操作と保存結果のハッシュ、照合失敗と上書き拒否を検証します。ローカルでも `python3 scripts/test-ai-edit-cli.py target/debug/ibisOffice` で実行できます。

## ステージング文書としての更新

Markdown の機能を追加または変更する PR では、利用者の表示や保存結果が変わる場合、関連する説明と最小の例を同じ PR で更新します。例には入力、期待する結果、確認した経路を添えます。既知の差は本文に残し、対応した時点で書き換えます。

公開できる合成例を使い、実文書で得た知見は再現できる最小例に置き換えます。構文や保存形式の回帰を検出する自動テストは、専用のテストにも追加します。この文書は機能を組み合わせたときの表示と操作を確認するために使います。
