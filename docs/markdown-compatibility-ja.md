# Markdownの参照仕様と互換性

ibisOfficeが受け付ける構文、通常の入出力での挙動、制限とサンプルは [Markdown構文リファレンス](markdown-reference-ja.md) にまとめています。[Markdown・YAML 入出力仕様](markdown-yaml-spec-ja.md) は各形式の判定と変換で保持する情報を定義します。このページは外部仕様の版、互換性の対象、相互運用性の方針、検証方法を記録します。

## 相互運用性の方針

Markdownの拡張は、次の方針で選びます。拡張を追加するときは、この方針に照らして書き方を決め、下の「[拡張の書き方と対応状況](#拡張の書き方と対応状況)」に記録します。

1. **CommonMarkとして正しい原稿にする。** 拡張を使った原稿も、CommonMark 0.31.2 とGFMの解釈で壊れないようにします。拡張を解釈しないビューアーでは、本文が読める形で残るようにします。
2. **既存の方言の書き方を選ぶ。** 同じ要求に応える記法が他の方言にあれば、新しい記法を作らずにそれを使います。読み込みでは主な変種を受け付け、書き出しでは一つの正規形を使います。正規形は、GitHubなどの表示に加え、VFM（Vivliostyle）で文字のまま描かれないかも確かめて選びます。
3. **意味を見えない注釈だけに持たせない。** 表のセル結合の `<!-- colspan:2 -->` のように、HTMLコメントは拡張を解釈しないビューアーで意味が隠れます。新しい拡張では、拡張を解釈しないビューアーでも内容と意図が読み取れる書き方を優先します。HTMLコメントは、Officeの書式を往復させるための補助に限ります。
4. **分野の語彙をibisOfficeに持ち込まない。** 文書の分野ごとの語（要件の種類、注意書きの区分など）は、ibisOfficeに組み込まず、原稿のフロントマターや共有の定義ファイルで利用者が定義します。フロントマターのキーは、既存の `x-ibis-format` と同じく `x-ibis-` を接頭辞にします。
5. **往復で正規形に戻す。** Markdown → DOCX → Markdown で正規形の原稿に戻り、AI向けJSONにも同じ意味が出るようにします。DOCXでは、意味を段落スタイルなどOfficeの要素に写します。
6. **曖昧さは規則で解く。** 方言の間で同じ書き方が別の意味を持つ場合（`:` で始まる段落は、Pandocでは表の表題、定義リストでは定義）、どちらに解釈するかの規則をこのページに記録します。

## 参照仕様

| 仕様・実装 | 対応している範囲 | 変換で注意する点 |
| --- | --- | --- |
| [CommonMark 0.31.2](https://spec.commonmark.org/0.31.2/) | ATX・Setext見出し、段落、リスト、引用、強調、リンク、画像、コードなどの基本構文 | 公式例を Markdown の読み込みと書き戻しの回帰検証に使う。編集可能性と DOCX 往復は個別に確認する。 |
| [GFM 0.29](https://github.github.com/gfm/) と [GitHubの書式ガイド](https://docs.github.com/en/get-started/writing-on-github/getting-started-with-writing-and-formatting-on-github/basic-writing-and-formatting-syntax) | パイプ表、タスクリスト、取り消し線、基本アラート | 表のセル結合やOfficeフィールドはibisOffice拡張。脚注とアラートはGFM形式仕様とは別にGitHubの書式ガイドを参照する。 |
| [MyST Markdown](https://mystmd.org/guide/syntax-overview) | Word互換の見出し・数式ラベル、表示名付き `{ref}` の同一文書内リンク、コードフェンス・コロンフェンス内の単一段落の基本admonition | コードフェンスとコロンフェンスの図は画像・代替テキスト・単一段落のキャプション・Word互換ラベルを取り込む。自動参照番号、引用、プロジェクトの参照解決は未対応。 |
| [Obsidian内部リンク](https://obsidian.md/help/links) と [callouts](https://obsidian.md/help/callouts) | 文書内Wikiリンク、開いたフォルダー内の別ファイル・見出しへのWikiリンク（大文字小文字無視・`aliases`・Vault内の近傍優先解決）、基本callout、原稿相対およびObsidian Vault内の画像埋め込み（`attachmentFolderPath`・Vault内探索） | Assistantのフォルダー参照は別ノートを名前またはルート基準のパスで引ける。`.obsidian/` があるフォルダーは `.git` 併存時もフォルダーペインを優先し、ヘッダーからGitビューへ切り替えられる。ノート埋め込み、折りたたみは未対応。 |
| [Marpit](https://marpit.marp.app/markdown) と [Marp Core](https://github.com/marp-team/marp-core/blob/main/docs/markdown.md) | スライド分割（ATX見出しの `headingDivider` を含む）、標準サイズ、発表者ノート、編集可能なページ番号のPPTX変換 | CSSテーマ、背景、元原稿への編集差分反映は未対応。 |
| [Mermaid](https://mermaid.js.org/) と [ZenUML](https://zenuml.com/docs/) | 図種別の描画、対応するグラフのOffice変換、対応する図ソースの再出力 | 図種と個別構文の対応は[構文リファレンス](markdown-reference-ja.md#mermaid)を参照。 |
| [Lawtext](https://github.com/yamachig/Lawtext) と [法令標準XML](https://laws.e-gov.go.jp/file/XMLSchemaForJapaneseLaw_v3.xsd) | 法令構造の変換と、元XMLへの限定的な編集反映 | 法令XMLを原本とし、対応を確認できない編集は診断で知らせる。 |
| [OKF v0.3](https://github.com/open-knowledge-format/open-knowledge-format/blob/main/SPEC.md) と [YAML 1.2.2](https://yaml.org/spec/1.2.2/) | ローカルのフォルダーで知識項目の認識、種類・タグによる絞り込み、信頼状態・鮮度の表示、参照元を指定した相対・ルート基準パスの解決 | 複数行のYAML値の編集は対象外。 |
| [Office Open XML ECMA-376](https://ecma-international.org/publications-and-standards/standards/ecma-376/) | DOCX、PPTX、XLSXの各変換経路 | アプリ間の見た目、文書モデルの往復、Markdownの元構文の復元は別々に検証する。 |
| [CommonMark generic directives（提案）](https://talk.commonmark.org/t/generic-directives-plugins-syntax/444) と [remark-directive](https://github.com/remarkjs/remark-directive) | `:::name[label]{attributes}` のコンテナの書き方を、利用者が定義するブロックの変種として読む（計画） | CommonMarkの仕様には採用されていない提案。書き出しの正規形には使わない。 |
| [Docusaurus admonitions](https://docusaurus.io/docs/markdown-features/admonitions) | `:::type[題名]` と、設定で登録する独自の種類の考え方 | 種類の登録はDocusaurusの設定ファイルで行う。ibisOfficeでは原稿のフロントマターで定義する。 |
| [Pandoc's Markdown](https://pandoc.org/MANUAL.html#pandocs-markdown) | fenced div `::: {.class}`、属性 `{#id .class key=val}`、パイプ表の区切り行による列幅の比、表の表題 `: …`、定義リスト、DOCXの `custom-style`（いずれも計画） | 区切り行の列幅はPandocでは行が `--columns` を超える場合に効く。ibisOfficeでの適用条件は下の表に記録する。 |
| [Vivliostyle Flavored Markdown（VFM）](https://github.com/vivliostyle/vfm/blob/main/docs/vfm.md) | ルビ `{漢字\|かんじ}` は同じ書き方。見出しのID `{#id}`、数式、脚注、GFMの表は共通の書き方を使う | 書き方の解釈の水準で互換を取る（下の「[VFMとの関係](#vfmとの関係)」）。 |
| [Quarto](https://quarto.org/docs/authoring/tables.html) | 表の表題と相互参照の `: 表題 {#tbl-…}`、列幅の `tbl-colwidths`（計画） | Quartoの相互参照はラベルの接頭辞 `tbl-` を要する。ibisOfficeの `{{ref:…}}` とは別に扱う。 |

新旧構文の互換性は、入力時に優先する構文と、変換後に書き出す構文の両方で決まります。OOMarkdownの目次と参考文献は `::: toc` と `::: bibliography` を使用し、Obsidianの二重角括弧にフィールドの意味を割り当てません。同一文書内のWikiリンクは通常のMarkdownアンカーになります。別ファイルへのWikiリンクはMarkdownへの再出力でもWiki記法を使います。

## 拡張の書き方と対応状況

長い構造化文書（仕様書、規程、教材など）で要る表現を、[相互運用性の方針](#相互運用性の方針)に沿って次の書き方で扱います。「拡張を解釈しない場合」は、CommonMarkとGFMだけを解釈するビューアーでの見え方です。

| 表現 | 書き出しの正規形 | 読み込みで受け付ける変種 | 拡張を解釈しない場合 | 状態 |
| --- | --- | --- | --- | --- |
| 引用・アラート・calloutの中の複数のブロック | `>` の中に、段落・リスト・コード・見出しをそのまま書く | GitHubのアラート、Obsidianのcallout、MySTのadmonition | 普通の引用（CommonMarkの引用は任意のブロックを含められる） | 対応済み |
| 利用者が定義するcalloutの種類と題名 | `> [!type] 題名`（Obsidianの形） | `> [!type]`（題名なし）、`> [!type]+` / `> [!type]-`（折りたたみの印は保持する） | 1行目が「[!type] 題名」の普通の引用 | 対応済み |
| calloutの種類の定義 | フロントマターの `x-ibis-blocks` | — | フロントマターなので本文に出ない | 対応済み |
| フロントマターの未知のキー | 原稿のとおり保持する | — | — | 対応済み |
| 表の列幅 | パイプ表の区切り行の `-` の数の比（Pandoc・Quarto） | Quartoの `tbl-colwidths` | 列幅を指定しない普通のパイプ表 | 計画 |
| 表の表題と番号 | 表に接する `: 表題 {#tbl_…}` の段落（Pandoc・Quarto） | 既存の `表 {{seq:table}} …` の段落 | 表の前後の普通の段落 | 計画 |
| 図の大きさ | 画像・図のフェンスの後の `{width=…}`（Pandoc） | — | 属性が文字として残る | 計画 |
| 目次の深さ | フロントマターの `toc-depth`（Pandoc・Quarto） | `::: toc {depth=2}` | フロントマターなので本文に出ない | 計画 |
| 定義リスト | `用語` の次の行に `: 定義`（Pandoc、PHP Markdown Extra） | — | 用語の段落と、「: 」で始まる段落 | 計画 |
| 別ファイルへのリンクのDOCX書き出し | 相対リンク `[文](other.md#見出し)` | Obsidianの `[[other#見出し\|文]]` | 普通のリンク | 計画 |

`:` で始まる段落は、表と空行一つ以内で接する場合だけ表の表題とし、それ以外は定義リストの定義として読みます。

### VFMとの関係

ibisOfficeはWord型の組版を第一とし、[VFM](https://github.com/vivliostyle/vfm/blob/main/docs/vfm.md)（Vivliostyle）の組版結果に寄せることはしません。VFMとは書き方の解釈の水準で互換を取り、VFMで書いた原稿をibisOfficeで開いたときに、次の三つが起きないようにします。

- **データを落とす**: 解釈できない書き方も、Markdownへの書き戻しで原稿のとおりに残す
- **書き方を文字のまま描く**: 解釈できない属性や囲みの印を、本文の文字として描かない
- **書き方を崩して描く**: 一部だけを別の書き方として解釈して、前後の本文まで崩さない

ibisOfficeの拡張の書き方を選ぶときも、VFMで文字のまま描かれにくい書き方を選びます。VFM向けの書き出しと、Vivliostyleによる組版は対象外です。

| 書き方 | VFMでの意味 | ibisOfficeの今の状態 | 目標 |
| --- | --- | --- | --- |
| ルビ `{漢字\|かんじ}` | ルビ | ルビ（同じ書き方） | — |
| 入れ子のルビ `{{東京\|とうきょう}\|Tokyo}` | 二重のルビ | 崩れて描かれる | VFMの次の版の規則で読む（「[日本語組版の記法とVFM](#日本語組版の記法とvfm)」） |
| 見出しのID `{#id}` | 見出しのID | Wordのブックマーク | — |
| 見出しのクラス `{.class}` | 見出しのクラス | 見出しの文字として描かれる（原稿には残る） | 描かず、原稿のとおり保持する |
| コードフェンスの題名 ```` ```js:app.js ````、`title=app.js` | コードブロックの題名 | 書き戻しで題名が落ちる。`js:app.js` は言語名も落ちる | 言語名と題名を分けて保持する |
| 画像の属性 `![題名](図.png){id="…"}` | 画像のID | 画像として描かれず、リンクの文字になる | 画像として描き、属性を保持する（`{width=…}` の計画と合わせる） |
| インラインの脚注 `^[…]` | 脚注 | 上付き文字として崩れ、後ろの脚注の参照も崩れる | 脚注として読む |
| 生のHTMLの囲み `<div class="…">` | HTMLの要素（中のMarkdownも解釈） | 囲みのタグが文字として描かれる（中のMarkdownは描かれる） | タグを描かず、原稿のとおり保持する |
| 数式 `$x = y$` | 数式 | 数式。書き戻しで空白が詰められる | 原稿の空白を保つ |
| フロントマターの `lang`・`class`・`vfm` | 文書の設定 | Markdown の保存と DOCX 往復で原稿のとおり保持する | — |
| `> [!type] 題名` | 普通の引用 | callout | — |
| `:::` の囲み | 段落の文字として残る（VFMには囲みの書き方が無い） | `::: toc`・`::: bibliography` など決まった語は正規形として書き出す | 利用者が定義するブロックの正規形には使わない |

この表の「今の状態」は、masanork ブランチ 21d912f7 で、上の書き方を並べた原稿を `--export md` と `--export png` にかけて確かめた結果です。

### 日本語組版の記法とVFM

ibisOfficeのMarkdown記法のうち、日本語組版に関わるもの（ルビ、傍点、縦中横、改行、図の題名）は、VFMに書き方がある場合はVFMに合わせます。VFMに書き方が無い場合はibisOfficeの書き方を使い、VFMの原稿で使われる生のHTMLの書き方も、ibisOfficeの解釈で吸収できる範囲で受け付けます。OOXMLとCSS組版の仕組みの違いから来る差（見出しによる節への分割、CSSのテーマ、改行・改ページの位置）は揃えません。

| 記法 | VFM | ibisOffice | 方針 |
| --- | --- | --- | --- |
| ルビ `{親文字\|ルビ}`、区切りのエスケープ `\\|` | あり | あり（同じ書き方。書き出しの正規形） | VFMと同じ意味で使う |
| 入れ子のルビ `{{東京\|とうきょう}\|Tokyo}` | 次の版で、内側から対にする（今の版は外側から） | 崩れて描かれる | VFMの次の版の規則（内側から）で読む（計画） |
| 青空文庫のルビ `｜親文字《ルビ》` | なし（文字として残る） | 読み込みで受け付ける | 書き出しはVFMと同じ `{親文字\|ルビ}` にする（今のまま） |
| 縦中横 `[25]{.tcy}`、傍点 `《《傍点》》`・`[傍点]{.em}` | 書き方なし（生のHTMLかCSSで行う） | ibisOfficeの書き方 | 今の書き方を保つ。生のHTMLの `<span class="tcy">`・`<span class="em">` も同じ意味で受け付ける（計画）。書き出しの正規形を変えるかは、upstreamの既存の原稿への影響を見て決める |
| 1つの改行 | `vfm: { hardLineBreaks: true }` で改行（既定は段落内の空白） | 段落内の空白 | フロントマターに `vfm.hardLineBreaks: true` があれば改行として読む（計画） |
| 1行の段落にある代替テキスト付きの画像 | 図と図の題名（`captionlessImagePolicy` で題名の無い画像の扱いを選ぶ） | 画像（代替テキストは表示しない） | 既存の原稿の見た目を変えないよう、フロントマターに `vfm` がある原稿だけVFMの規則で図の題名にする（計画） |
| インラインの脚注 `^[…]` | 脚注 | 上付き文字として崩れる | 脚注として読む（計画） |
| 縦書き | CSSのテーマ・クラスで行う | フロントマターの `direction: vertical` | 仕組みが違うので揃えない。`class` などVFMのキーは保持する |

### calloutの種類の定義

calloutの種類は、GitHubの5種（`note`、`tip`、`important`、`warning`、`caution`）と、Obsidianの別名（`hint` は `tip`、`attention` は `warning`）を既定で持ちます。それ以外の種類は、フロントマターの `x-ibis-blocks` で題名と見た目を定義できます。

```yaml
x-ibis-blocks:
  must: { title: 実装必須機能, tone: important }
  option: { title: 標準オプション機能, tone: note }
```

`title` は題名を省いたときに表示する題名、`tone` は見た目（既定の5種のどれに準じるか）です。定義の無い種類は、Obsidianと同じく `note` の見た目で、種類名を題名として表示します。原稿に題名を書いた場合は、定義の題名より優先します。DOCXでは種類・折りたたみの印・明示した題名を題名段落のスタイルと内容に、本文のまとまりを引用の継続スタイルに保持します。折りたたみの印は保存・再読込で保持し、ibisOffice では本文を展開して表示します。

## OKFとフォルダーの読み取り

フォルダーペインとAssistantの `folder_list` は `type:Metric` と、個別タグに完全一致する `tag:finance` を共有します。拡張子 `.md` のファイルで、先頭のフロントマターに文字列の `type` がある場合に知識項目として扱います。`index.md` と `log.md` は知識項目から除外し、一般ファイルとして開けます。種類はフォルダーを開いたときに読み取り、フロントマターの探索はファイルあたり64 KiBまでです。

Assistantの `folder_read` は任意の `from` に参照元ファイルを指定できます。たとえば `from: metrics/revenue.md` と `path: ../README.md` はルートのREADMEを読みます。`path: /docs/method.md` は開いたフォルダーのルートを基準にします。同じ操作で `[[method]]` や `[[docs/method]]` も読めます。同名の候補がある場合はパス指定が必要です。フォルダーペインから開いたMarkdownでは、本文の通常の相対リンクとルート基準リンクをたどれます。リンク先の明示ブックマークや見出しIDが見つかれば、その段落に移動します。デスクトップで開いた同じMarkdown文書への見出しリンクは通常のアンカー記法とWikiリンクの両方から移動できます。デスクトップのフォルダーペインから開いた原稿では、`[[手順]]` や `[[資料/手順#詳細|開く]]` からも対象のファイル・見出しへ移動できます。

Gitワークツリー内のフォルダーでは、Assistantの `git_read` に `git_log` で得たコミットSHAと参照元を渡せます。参照元とリンク先の両方を同じコミットから読み、リンク先のフラグメントを返します。作業ツリーの内容への切り替えは行いません。履歴ビューで単一コミットのMarkdownを開いた場合も、本文リンクから同じコミットのファイルと見出しに移動できます。Wikiリンクはリポジトリ基準のパス、またはツリー内で一意なファイル名を使えます。

フォルダーを開くときと再読み込みするときに、Markdownのメタデータ・タグ（フロントマターの `tags` および本文のインライン `#tag`、階層タグ前方一致）・`aliases`・バックリンクの索引をアプリデータ配下の Turso（SQLite）キャッシュで差分更新します（走査上限の10,000ファイルまで対象）。本文の全文検索キャッシュは先頭256ファイル・合計8 MiBをメモリ上に保持し、ファイルの更新時刻やサイズが変わった場合、またはキャッシュに収録されなかった場合は元ファイルを直接読みます。

## 形式指定と知識項目のメタデータ

新しいLawtextとスライド書き出しでは `x-ibis-format` に形式を明示します。以前の `type: lawtext`、`type: presentation` などは読み込み時に引き続き認識します。知識項目の型が従来の形式名と同じ場合、`x-ibis-format: document` で文書として開けます。

文書経路で読み込んだ `type` のあるMarkdownでは、YAMLブロックをDOCXのカスタムプロパティに保存します。DOCXのタイトル・説明・キーワードを編集してMarkdownへ戻すと、元の `title`・`description`・`tags` のうち変更した値を反映します。キーワードはカンマ区切りで入力します。未知の入れ子キーとコメントは保持します。複数行にまたがる値やカンマを含む既存タグなど、安全に更新できない値は原稿に残し、Markdown出力の診断にキーを記録します。以前のビルドのDOCXに保存されたYAMLは、従来どおり取り込み時の値を保持します。

## 検証

CommonMark の公式例と比較する場合は、ibisOffice で読み込んだ文書を Markdown に書き戻し、参照実装の cmark の描画結果を比較する。この比較には読み込みと書き戻しの両方が含まれる。CommonMark 0.31.2 の公式652例をこの手順で検証し、cmark 0.31.2 の出力を正規化して比較した結果、652例すべてが一致した。編集可能性は、文書モデルのリンク先とタイトル、および DOCX 往復でも確認する。

単一行の参照形式リンクは編集可能なリンクとして取り込み、書き戻し時はリンク先とタイトルを含むインライン形式にする。DOCX ではタイトルを ScreenTip として保持する。参照形式の画像はデータ URI、またはデスクトップで原稿フォルダー内の相対パスから読み込める。保存時は画像データを Markdown に埋め込み、代替テキストとタイトルを保持する。ネットワーク上の画像は取得しない。複数行にまたがる参照や入れ子のリンクラベルには引き続き制限がある。

文書直下・引用内・リスト項目内の `<details>`・`<summary>`、`<div>`・`<section>`、HTML の表、`<pre>`、通常の HTML コメントなどは、編集可能なソースとして取り込む。Markdown に戻すとタグ・属性・ブロック内の改行と字下げを保持し、引用やリスト項目の内側に書き戻す。DOCX 上では等幅のソースとして表示する。

Setext 見出しは編集可能な見出しとして読み込み、書き戻し時は ATX 記法（`#` または `##`）を使う。通常の番号付きリストでは先頭項目の番号を保持する。段落途中の行にある `14.` などは本文として扱い、リスト項目間の空行は書き戻す。文書直下と引用内では、字下げコードを本文の空白を保ったままコードフェンスに書き戻す。見出し・段落・通常のコードブロックからなる引用は、各ブロックを引用内に保つ。通常のコードフェンスでは、短い言語名（`python` や `C++` など）と本文を保持する。リスト項目の継続段落、字下げコード、通常のコードフェンスは、親項目の下に字下げして書き戻す。コードフェンスの短い言語名も保持する。段落・見出し・対応する HTML やコードを含む引用の入れ子は、8階層まで引用の深さと区切りを保つ。引用内の箇条書きと通常の番号付きリストは、開始番号・入れ子・項目間の空行を保持する。リスト項目内で始まる引用も、段落・対応する HTML・引用の入れ子を親項目の中に保つ。リスト項目内や引用内の GFM 表は編集可能な表として読み込む。見出し行・セル内容・列揃えを保持し、元のリスト項目や引用の階層に書き戻す。リスト項目内の引用にある表も、項目の字下げと引用の深さを保持する。

- `cargo test -p ibis-formats --lib markdown::tests` で構文とOffice変換の回帰を確認する。
- `cargo test -p ibis-formats --lib okf::tests` で概念の判定とOKFパス規則を確認する。
- `cargo test -p ibis-office-shell --lib folder_context::tests` でローカルフォルダー内の一覧と参照読取を確認する。

互換機能を追加するときは、入力構文、書き出し時の構文、原本へ反映できる編集の範囲を記録し、該当の回帰テストを更新します。
