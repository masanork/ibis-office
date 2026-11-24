# Markdownの参照仕様と互換性

ibisOfficeが受け付ける構文、通常の入出力での挙動、制限とサンプルは [Markdown構文リファレンス](markdown-reference-ja.md) にまとめています。[Markdown・YAML 入出力仕様](markdown-yaml-spec-ja.md) は各形式の判定と変換で保持する情報を定義します。このページは外部仕様の版、互換性の対象、検証方法を記録します。

## 参照仕様

| 仕様・実装 | 対応している範囲 | 変換で注意する点 |
| --- | --- | --- |
| [CommonMark 0.31.2](https://spec.commonmark.org/0.31.2/) | ATX・Setext見出し、段落、リスト、引用、強調、リンク、画像、コードなどの基本構文 | 公式例を Markdown の読み込みと書き戻しの回帰検証に使う。入れ子、HTML、参照リンクには既知の差がある。 |
| [GFM 0.29](https://github.github.com/gfm/) と [GitHubの書式ガイド](https://docs.github.com/en/get-started/writing-on-github/getting-started-with-writing-and-formatting-on-github/basic-writing-and-formatting-syntax) | パイプ表、タスクリスト、取り消し線、基本アラート | 表のセル結合やOfficeフィールドはibisOffice拡張。脚注とアラートはGFM形式仕様とは別にGitHubの書式ガイドを参照する。 |
| [MyST Markdown](https://mystmd.org/guide/syntax-overview) | Word互換の見出し・数式ラベル、表示名付き `{ref}` の同一文書内リンク、コードフェンス・コロンフェンス内の単一段落の基本admonition | コードフェンスとコロンフェンスの図は画像・代替テキスト・単一段落のキャプション・Word互換ラベルを取り込む。自動参照番号、引用、プロジェクトの参照解決は未対応。 |
| [Obsidian内部リンク](https://obsidian.md/help/links) と [callouts](https://obsidian.md/help/callouts) | 文書内Wikiリンク、開いたフォルダー内の別ファイル・見出しへのWikiリンク、基本callout、原稿からの相対パスで指定した画像埋め込み | Assistantのフォルダー参照は別ノートを名前またはルート基準のパスで引ける。ノート埋め込み、折りたたみは未対応。 |
| [Marpit](https://marpit.marp.app/markdown) と [Marp Core](https://github.com/marp-team/marp-core/blob/main/docs/markdown.md) | スライド分割（ATX見出しの `headingDivider` を含む）、標準サイズ、発表者ノート、編集可能なページ番号のPPTX変換 | CSSテーマ、背景、元原稿への編集差分反映は未対応。 |
| [Mermaid](https://mermaid.js.org/) と [ZenUML](https://zenuml.com/docs/) | 図種別の描画、対応するグラフのOffice変換、対応する図ソースの再出力 | 図種と個別構文の対応は[構文リファレンス](markdown-reference-ja.md#mermaid)を参照。 |
| [Lawtext](https://github.com/yamachig/Lawtext) と [法令標準XML](https://laws.e-gov.go.jp/file/XMLSchemaForJapaneseLaw_v3.xsd) | 法令構造の変換と、元XMLへの限定的な編集反映 | 法令XMLを原本とし、対応を確認できない編集は診断で知らせる。 |
| [OKF v0.3](https://github.com/open-knowledge-format/open-knowledge-format/blob/main/SPEC.md) と [YAML 1.2.2](https://yaml.org/spec/1.2.2/) | ローカルのフォルダーで知識項目の認識、種類・タグによる絞り込み、信頼状態・鮮度の表示、参照元を指定した相対・ルート基準パスの解決 | 複数行のYAML値の編集は対象外。 |
| [Office Open XML ECMA-376](https://ecma-international.org/publications-and-standards/standards/ecma-376/) | DOCX、PPTX、XLSXの各変換経路 | アプリ間の見た目、文書モデルの往復、Markdownの元構文の復元は別々に検証する。 |

新旧構文の互換性は、入力時に優先する構文と、変換後に書き出す構文の両方で決まります。OOMarkdownの目次と参考文献は `::: toc` と `::: bibliography` を使用し、Obsidianの二重角括弧にフィールドの意味を割り当てません。同一文書内のWikiリンクは通常のMarkdownアンカーになります。別ファイルへのWikiリンクはMarkdownへの再出力でもWiki記法を使います。

## OKFとフォルダーの読み取り

フォルダーペインとAssistantの `folder_list` は `type:Metric` と、個別タグに完全一致する `tag:finance` を共有します。拡張子 `.md` のファイルで、先頭のフロントマターに文字列の `type` がある場合に知識項目として扱います。`index.md` と `log.md` は知識項目から除外し、一般ファイルとして開けます。種類はフォルダーを開いたときに読み取り、フロントマターの探索はファイルあたり64 KiBまでです。

Assistantの `folder_read` は任意の `from` に参照元ファイルを指定できます。たとえば `from: metrics/revenue.md` と `path: ../README.md` はルートのREADMEを読みます。`path: /docs/method.md` は開いたフォルダーのルートを基準にします。同じ操作で `[[method]]` や `[[docs/method]]` も読めます。同名の候補がある場合はパス指定が必要です。フォルダーペインから開いたMarkdownでは、本文の通常の相対リンクとルート基準リンクをたどれます。リンク先の明示ブックマークや見出しIDが見つかれば、その段落に移動します。デスクトップで開いた同じMarkdown文書への見出しリンクは通常のアンカー記法とWikiリンクの両方から移動できます。デスクトップのフォルダーペインから開いた原稿では、`[[手順]]` や `[[資料/手順#詳細|開く]]` からも対象のファイル・見出しへ移動できます。

Gitワークツリー内のフォルダーでは、Assistantの `git_read` に `git_log` で得たコミットSHAと参照元を渡せます。参照元とリンク先の両方を同じコミットから読み、リンク先のフラグメントを返します。作業ツリーの内容への切り替えは行いません。履歴ビューで単一コミットのMarkdownを開いた場合も、本文リンクから同じコミットのファイルと見出しに移動できます。Wikiリンクはリポジトリ基準のパス、またはツリー内で一意なファイル名を使えます。

フォルダーを開くときと再読み込みするときに、Markdown本文の検索索引をメモリ上に再構築します。Assistantのフォルダー検索は索引から元の行番号を返します。索引は最大256ファイル・本文と検索用文字列の合計8 MiBです。ファイルの更新時刻やサイズが変わった場合、または索引に収録されなかった場合は元ファイルを直接読みます。

## 形式指定と知識項目のメタデータ

新しいLawtextとスライド書き出しでは `x-ibis-format` に形式を明示します。以前の `type: lawtext`、`type: presentation` などは読み込み時に引き続き認識します。知識項目の型が従来の形式名と同じ場合、`x-ibis-format: document` で文書として開けます。

文書経路で読み込んだ `type` のあるMarkdownでは、YAMLブロックをDOCXのカスタムプロパティに保存します。DOCXのタイトル・説明・キーワードを編集してMarkdownへ戻すと、元の `title`・`description`・`tags` のうち変更した値を反映します。キーワードはカンマ区切りで入力します。未知の入れ子キーとコメントは保持します。複数行にまたがる値やカンマを含む既存タグなど、安全に更新できない値は原稿に残し、Markdown出力の診断にキーを記録します。以前のビルドのDOCXに保存されたYAMLは、従来どおり取り込み時の値を保持します。

## 検証

CommonMark の公式例と比較する場合は、ibisOffice で読み込んだ文書を Markdown に書き戻し、参照実装の cmark の描画結果を比較する。この比較には読み込みと書き戻しの両方が含まれる。たとえば参照リンクの構文が文字列として残っている場合、cmark が書き戻し後に解決できても、Office の画面でリンクを操作できることは確認できない。

Setext 見出しは編集可能な見出しとして読み込み、書き戻し時は ATX 記法（`#` または `##`）を使う。通常の番号付きリストでは先頭項目の番号を保持する。文書直下と、コードブロック単独の引用では、字下げコードを本文の空白を保ったままコードフェンスに書き戻す。通常のコードフェンスでは、短い言語名（`python` や `C++` など）と本文を保持する。リスト項目内の複数ブロックや引用内の見出しの保持には制限がある。

- `cargo test -p ibis-formats --lib markdown::tests` で構文とOffice変換の回帰を確認する。
- `cargo test -p ibis-formats --lib okf::tests` で概念の判定とOKFパス規則を確認する。
- `cargo test -p ibis-office-shell --lib folder_context::tests` でローカルフォルダー内の一覧と参照読取を確認する。

互換機能を追加するときは、入力構文、書き出し時の構文、原本へ反映できる編集の範囲を記録し、該当の回帰テストを更新します。
