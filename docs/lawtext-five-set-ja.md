# 法令XML・Lawtextから法案五点セットを起案する

法律案の五点セットは、法律案の条文、理由、要綱、新旧対照表、参照条文で構成されます。Docsは、同一法令の現行版と改正後の溶け込み版から、五点の編集可能な草案を作ります。対象は一つの法令の一部改正です。複数法令を改正する法案は、対象法令ごとに差分を作り、法案全体の構成は担当者が組み立てます。[参議院「法令の改正方式（改め文、新旧対照表）」](https://www.sangiin.go.jp/japanese/annai/chousa/keizai_prism/backnumber/r03pdf/202120102.pdf)

## 入力を用意する

- **現行版**は、立案の基準日に適用する法令本文です。基準日と取得元を控えておきます。
- **改正後版**は、同じ法令に変更を反映した本文です。改め文を入力する場合は、別途、逆適用検算に渡します。
- **担当者メモ**には、制度上の課題、変更の狙い、根拠資料を記載します。
- **参照資料**は、外部法令の適用時点に対応する本文を必要に応じて用意します。

ファイルは UTF-8 の e-Gov 法令標準XML、Lawtext、または Docs の `x-ibis-format: lawtext` 付き Markdown（旧 `type: lawtext` も読み込み可） を使えます。現行と改正後で異なる形式も使えます。法令番号が双方にある場合は表記もそろえます。

## コマンドで一括生成する

リポジトリのルートで実行します。次の例は、改正後列を左に配置し、五点と確認結果を一つの Markdown に出力します。メモと参照資料は任意です。`--reference` は繰り返し指定できます。

```sh
cargo run --locked -p ibis-formats --bin ibis-lawtext-5set -- \
  current.xml amended.lawtext \
  --memo policy.md --reference cited-law.xml > bill-drafts.md
```

`--old-on-left` を付けると現行列が左になります。出力ファイルは Docs で開いて編集できます。新旧対照表の傍線を確認する場合は、Docs の Markdown 表示を使います。

五点を個別のファイルとして扱う場合は JSON を出力し、`documents` の各値を取り出します。

```sh
cargo run --locked -p ibis-formats --bin ibis-lawtext-5set -- \
  current.xml amended.lawtext --json --require-match > bill-drafts.json

jq -r '.documents.youkou' bill-drafts.json > youkou.md
jq -r '.documents.bill' bill-drafts.json > bill.md
jq -r '.documents.riyuu' bill-drafts.json > riyuu.md
jq -r '.documents.shinkyu' bill-drafts.json > shinkyu.md
jq -r '.documents.sansho' bill-drafts.json > sansho.md
```

通常は草案を書き出せれば終了コードは `0` です。`--require-match` を指定した場合、逆適用が一致しなければ草案を書き出したうえで `2` を返します。入出力や解析のエラーは `1` です。JSON の `deterministic_outputs` から残差の構造化差分、逆適用の診断、lint結果を確認できます。

公開済みの改正を再現する際は、`--amendment-law-num '令和八年七月三一日法律第七六号'` のように、改正後XMLの `SupplProvision` にある `AmendLawNum` の値を指定できます。指定番号が改正後の版に新設された附則を一意に示す場合、検算はその附則を改正法案の確認事項として表示し、ほかの残差と区別します。JSONでは `bill_supplement_law_num` に番号が入り、`residual_diff` はそれ以外の差分を示します。全体の一致を求める `--require-match` は、この附則が残る場合も終了コード `2` を返します。

## 公開例の組版に合わせたセット版を DOCX と PDF にする

`--docx` は要綱、法律案、理由、新旧対照条文、参照条文をこの順に収録した一つの DOCX を出力します。各文書は新しいページから始まります。厚生労働省の第221回国会提出資料のうち「予防接種法の一部を改正する法律案」を組版の参照例としました。[要綱](https://www.mhlw.go.jp/content/001709107.pdf) · [法律案・理由](https://www.mhlw.go.jp/content/001709112.pdf) · [新旧対照条文](https://www.mhlw.go.jp/content/001709113.pdf) · [参照条文](https://www.mhlw.go.jp/content/001709114.pdf)

| 文書 | 用紙と組方向 |
| --- | --- |
| 要綱 | A4縦判・横組み一段 |
| 法律案・理由 | A4縦判・右から左への縦組み。理由を改ページ |
| 新旧対照条文 | A4縦判・縦組み。上段が改正案、下段が現行。黒罫と両段の傍線を使用 |
| 参照条文 | A4横判・横組み二段 |

`--old-on-left` を付けると新旧対照条文の上段が現行になります。この構成は選んだ公開例に合わせています。目次、頁番号の位置、抜粋範囲、提出先ごとの細則は DOCX で調整できます。

```sh
mkdir -p out
cargo run --locked -p ibis-formats --bin ibis-lawtext-5set -- \
  current.xml amended.lawtext --memo policy.md --reference cited-law.xml \
  --docx > out/bill-set.docx
cargo run --locked -p ibis-office-app --bin ibisOffice -- \
  --export pdf --out-dir out out/bill-set.docx
```

同じ DOCX から `out/bill-set.pdf` を生成します。Docs のデスクトップ版で DOCX を開き、編集後にPDFへ書き出すこともできます。指定書体は Noto Serif CJK JP です。PDFを作る環境ではこの書体を利用できるようにし、改ページ、傍線、罫線、文字のはみ出しを確認してください。lint と逆適用の診断一覧は上記の Markdown または JSON 出力で確認できます。

## 字詰めとOOXML互換性

2021年4月2日の衆議院内閣委員会で、内閣法制局は閣議請議の法律案・政令案について、縦書き・一行48字・一ページ13行詰めと説明しました。同じ答弁で、Wordの使用に問題はなく、法令審査支援システムはWordと一太郎の両方に対応していると述べています。デジタル庁の調査では、字下げ、禁則、字数・行数、行間の調整が法制執務の負担として挙げられています。[衆議院内閣委員会会議録](https://www.shugiin.go.jp/internet/itdb_kaigiroku.nsf/html/kaigiroku/000220420210402014.htm) · [デジタル庁調査](https://www.digital.go.jp/assets/contents/node/basic_page/field_ref_resources/76a86fd3-eec1-42c5-bf42-6cc3fe32e9da/99a9a6c0/20231027_meeting_digital-system-reform-wg-legal-practice_outline_04.pdf)

このセット版では案文と理由に、公表PDFから観測した約14.05ポイントの字送り、34ポイントの行送りを設定しています。本文領域は全角48字・縦13行分です。別の文書である要綱、新旧対照条文、参照条文には、このグリッドを適用しません。

DOCXでは標準のセクション設定、`w:textDirection`、`w:docGrid`（`linesAndChars`、`linePitch`、`charSpace`）、段落の両端揃えを使います。文字グリッドの基準となるNormalスタイルは14ポイントです。文字列は編集可能なままです。和欧混植、約物の禁則・追い込み、ルビ、フォント代替、WordとDocsの版差で行末や改ページが変わり得ます。48字は全角換算の枠であり、各行に48個のUnicode文字を入れる指定ではありません。Wordでの印刷結果と提出用PDFを照合してください。[OOXML文書グリッド](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.docgrid?view=openxml-3.0.1) · [グリッドへの文字配置](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.snaptogrid?view=openxml-3.0.1)

## 実法案を探して検証する

`houritsu` の Git 履歴は、施行時点ごとの改正箇所、e-Gov の版ID、改正の対象法令、衆議院の議案情報を結び付ける索引に使えます。コミットと親の差分を読むと候補を絞れます。Markdownの front matter や表の表現は `houritsu` 独自なので、Docsへの入力にはそこに書かれた版IDから取得した **e-Gov 法令標準XML** を使います。提出時の案文・理由・要綱と正式な新旧対照条文は提出府省の資料で照合します。段階施行や同日施行があるときは、対象法令ごとに前後のXMLを確認します。[houritsuの記事](https://zenn.dev/koheiwada/articles/houritsu-japanese-laws-as-git) · [e-Gov法令API](https://laws.e-gov.go.jp/api/2/swagger-ui)

実例の「予防接種法の一部を改正する法律案」は、第221回国会の閣法第63号、公布法は令和8年法律第76号です。`houritsu` の改正コミットから、予防接種法と新型インフルエンザ等対策特別措置法の二法が対象と確認できます。予防接種法の比較には次のXMLを使用しました。日付は版の識別子として記載しています。施行日は公布法・政令で確認してください。[houritsuの改正コミット](https://github.com/Kohei-Wada/houritsu/commit/741ba6ebb75fea3105fbaf8e0455d51ad63eb8ed) · [衆議院の議案情報](https://www.shugiin.go.jp/internet/itdb_gian.nsf/html/gian/honbun/g22109063.htm)

| 版 | e-Gov 法令改正履歴ID |
| --- | --- |
| 改正前 | `323AC0000000068_20261001_507AC0000000087` |
| 改正後 | `323AC0000000068_20270130_508AC0000000076` |

API の `GET /api/2/law_data/{law_revision_id}?response_format=xml` から各版を取得し、応答の `<Law>` 要素を取り出して入力します。七つの本則条文が差分として検出され、公開新旧対照条文の本則の対象と一致しました。今回の自動生成は予防接種法の部分を対象とする草案です。逆適用検算では本則の差分は解消され、附則の差分が残りました。別法の改正と実際の要綱・理由は厚生労働省の提出資料で確認できます。[提出資料一覧](https://www.mhlw.go.jp/stf/topics/bukyoku/soumu/houritu/221.html)

## 文書画面で新旧対照条文を作る

1. デスクトップ版で改正後の法令文書を開き、**校閲 → 比較 → 文書の比較…** を選びます。法令文書では、ファイル名のメニューに **新旧対照条文を作成…** も表示されます。
2. 現行版の法令XML、Lawtext、法令Markdownを選びます。確認画面でファイル名と、取得できた法令名・法令番号を確認します。前後を入れ替えることもできます。
3. **文書として開く**で、縦組み・上下二欄の新旧対照条文が新しい文書ウィンドウに開きます。編集後、DOCXとして保存するか、PDFに書き出せます。

ブラウザ版では、改正後の法令XMLまたはLawtextを開いて **校閲 → 比較 → 文書の比較…** を選び、手元の現行版を指定します。確認画面の **DOCXをダウンロード** で結果を保存できます。ブラウザ版は元の法令ファイル同士を比較する機能です。作業中の編集を比較に含める場合はデスクトップ版を使用します。

比較画面は現時点で法令の構造に対応しています。デスクトップ版では作業中の編集を含む本文を比較します。この操作はAI接続を必要としません。

## Git履歴の二版から新旧対照条文を作る

デスクトップ版では、ローカルのGitリポジトリにコミットされた法令XMLまたはLawtextを比較できます。履歴の閲覧と比較にはシステムのGitを使います。

1. **ファイル → リポジトリを開く…** でローカルのリポジトリを選びます。現在のHEADより前の版を起点にファイルを探す場合は、**履歴** でコミットを選ぶか、IDを入力して **コミットへ移動** を押します。
2. ファイル一覧で対象の法令を選びます。ファイル履歴で現行に **旧版**、改正後に **新版** を指定します。別ブランチなど、ファイル履歴に表示されていない現行版を使う場合は、**比較元のコミットID** に完全なIDまたは一意に絞れる4桁以上の短縮IDを入力します。旧版のファイル名や配置が異なる場合は、**旧版のパス（任意）** にリポジトリのルートからの相対パスも入力します。最後に **旧版に指定** を押します。
3. 画面に表示された旧版と新版のIDを確認し、**二版を比較** を押します。上段に改正案、下段に現行を配置した縦組み・罫線付きの新旧対照条文が、新しいDOCX文書として開きます。
4. **ファイル → 名前を付けて保存…** でDOCXを保存できます。保存名の候補には旧版と新版のコミットIDが入ります。PDFは **ファイル → 書き出し → PDF（.pdf）** で書き出せます。先にDOCXを保存する必要はありません。

旧版を履歴から見つけた場合は、対象ファイルの履歴で該当する行の **候補に登録** を押します。そのコミットを開いてファイル一覧から **旧版候補に登録** を押すこともできます。同じリポジトリで新版のコミットとファイルを開き、**候補を旧版に指定** を押すと、登録した旧版のコミットIDとパスが使われます。

比較対象はコミットに記録された本文です。未コミットの編集を含めたい場合は、上の「文書画面で新旧対照条文を作る」の手順を使います。Git履歴からの比較で作成する文書は新旧対照条文です。

### 実例での比較・出力確認

上記の予防接種法の二版を検証用リポジトリにコミットし、Linux版のファイル履歴から選択して単体の新旧対照条文を保存しました。ibisOfficeのPDFはA4縦判で6ページ、同じDOCXをLibreOfficeで開いた結果は5ページでした。いずれも縦組み、上下二欄、罫線を保持しています。短い附則は見出しと本文を同じ表の行にまとめ、改ページで分割しないようにします。この実例ではibisOfficeの6ページ目に附則全体が載ります。Windows版のWordでの改ページは未確認です。[公開の新旧対照条文](https://www.mhlw.go.jp/content/001709113.pdf)

## Docs の AI Assistant で起案する

1. デスクトップ版の文書画面で改正後本文を開きます。
2. AI Assistant を開き、法令名・法令番号・法令履歴IDのいずれかを伝えて、e-Gov から現行版を取得し比較元に設定するように依頼します。
3. 目的、必要性、施行期日など判明している事項を指示に含めます。各草案を開く必要がある場合は、Markdown文書として開くよう依頼します。
4. 改め文を加筆した場合は、現行版と改正後版を使って再検算を依頼します。

AI Assistant の `generate_lawtext_5set` は五点の草案、構造化差分、検算結果、lint結果、起案用プロンプトを返します。外部法令本文を渡せる場合は `reference_laws` で参照条文に差し込めます。ローカル完結で処理する場合は上記のコマンドを使います。

### 改正対象の候補を調べる

対象が未確定の場合は、基準日と公表済みの検索語を伝えます。例えば「2023年3月6日時点で『磁気ディスク』を含む条文を検索し、申請者に媒体の提出を求める規定の候補を出典付きで挙げてください」と依頼できます。検索語は e-Gov の公開 API に送られます。

`search_egov_law` は検索結果の抜粋、位置、法令履歴ID、その版の法令XMLへのURLを返します。位置が本則などの大まかな範囲にとどまる場合は、該当する版の本文で条項を確認します。検索結果は参考候補で、改正対象やハネ改正の網羅性は保証しません。比較元として採用した版は、`fetch_egov_law` に法令履歴IDを指定して取得できます。

## 提出前に確認する

| 草案 | 自動生成される内容 | 担当者の確認 |
| --- | --- | --- |
| 要綱 | 題名、条の追加・削除・改正・移動、編章、附則、別表の差分に基づく骨子 | 制度の内容と趣旨、重要事項の取捨選択 |
| 法律案 | 改め文と附則の記入欄 | 適用順、改め文の定型、施行期日、経過措置、関係法令の整備 |
| 理由 | 対象法案、担当者メモ、理由の記入欄と結語 | 改正の必要性、根拠、提出用の文章 |
| 新旧対照表 | 解析できた条文の対照と変更箇所の強調 | 改正対象範囲、傍線、列順、提出先書式 |
| 参照条文 | 改正後の変更条文から検出した本則の引用候補と、解決できた条文 | 引用の網羅性、外部法令の適用時点、掲載する範囲 |

逆適用検算は、起案した改め文を現行版に適用し、解析後の題名・制定文・前文・本則・附則・別表を比較します。施行期日など改正法案自体に置く附則は、担当者が根拠資料に沿って起案します。検算が一致しても、XMLの未対応要素、政策判断、提出先の様式は確認が必要です。

外部法令は検出できた引用の法令番号、または法令名で指定資料と照合します。版が複数一致する場合や対象条文が見つからない場合は、参照条文に確認事項を表示します。略称、範囲指定、附則・別表をまたぐ引用は原文を確認してください。表・図・様式や入れ子構造が複雑な場合も、取込後の構造を原文と照合します。対応する構文は [Markdown・Mermaid・Lawtext リファレンス](markdown-reference-ja.md) を参照してください。

## 法令XMLの変換診断

`ibis-lawtext-audit` に法令XMLの `<Law>` 要素を含むファイルを渡すと、構造差をJSONで確認できます。公式スキーマも検証する場合は、ローカルに保存したXSDと `xmllint` を使います。 [e-Gov 法令標準XMLスキーマ](https://laws.e-gov.go.jp/docs/law-data-basic/419a603-xml-schema-for-japanese-law/)

```sh
cargo run --locked -p ibis-formats --bin ibis-lawtext-audit -- \
  current.xml --xsd XMLSchemaForJapaneseLaw_v3.xsd > audit.json
```

- `xml_model_changes` は原XMLと、ibisOfficeの内部表現から書き出したXMLの差です。
- `lawtext_roundtrip_changes` は内部表現から出したXMLと、Lawtextを経由した後のXMLの差です。
- `markdown_source_diagnostics` は編集用Markdownから復元できなかった元ソースの位置と理由です。
- `markdown_roundtrip_changes` は内部表現から出したXMLと、編集用Markdownを経由した後のXMLの差です。
- `output_xsd` はLawtext経由、`markdown_output_xsd` は編集用Markdown経由のXMLのスキーマ検証結果です。`--xsd` を省略した場合は両方とも `null` になります。どちらかが不適合なら、JSONを書き出して終了コード `2` を返します。実行エラーは `1` です。

差分の位置には条・項・号の `Num`、附則の `AmendLawNum` と、同じ位置キーの出現順を用います。診断は要素・属性・テキストの差を示します。対応付けた要素の相対順が変わった場合は、親要素の位置に `child_order_changed` を表示します。同じ識別子の要素や識別子を持たない同種の要素は出現順で対応付けます。属性順と整形用の空白は比較対象外です。要素を失った場合は、その配下をまとめて親要素の位置に表示します。


### 法令XMLの原本へ編集を反映する

`ibis-lawtext-apply` は編集中のファイルとは別に、編集開始時に使った法令XMLの原本を入力に取ります。編集用ファイルを生成し、編集後に別の出力先へ候補XMLを書き出します。

```sh
ibis-lawtext-apply original.xml --prepare --format markdown > draft.md
ibis-lawtext-apply original.xml draft.md > candidate.xml
```

Lawtextを使うときは生成時に `--format lawtext` を指定し、拡張子を `.law.txt` にします。編集開始から反映まで、同じ版の `original.xml` を保管します。リダイレクト先には原本と異なるパスを指定してください。

原本で対応を確定できる平文の `Sentence` の変更を反映します。別表の単一平文セルにも適用できます。変更箇所のテキストだけを差し替え、元の属性、コメント、整形、未対応要素を保持します。行や条の追加・削除、ルビなどの混在する文、曖昧な並べ替え、元XMLの対応を確定できない編集があれば、全件を保留して診断を標準エラーにJSONで返します。保留時にXMLは出力せず、終了コードは `2` です。変換後のXSD検証は `ibis-lawtext-audit` で行えます。

生成するLawtextはibisOfficeの既存の読み書き規則に従います。書き戻しには編集開始時に生成した形式とメタデータを維持してください。本家CLIによる別レイアウトへの再整形も、再現性を確認できなければ保留します。

XMLから直接読み込んだ項・号・号の細分は、本文が同じ間、複数の `Sentence` の境界と `Num`、`Function`、`Indent`、`WritingMode` をXML書き出し時に保持します。改め文の生成・検算には従来どおり連結した本文を用います。ibisOfficeのLawtextは項・号・号の細分の本文に `<Sentence Function="main">…</Sentence><Sentence Function="proviso">…</Sentence>` と、文ごとのXML要素を記録できます。この記法はibisOfficeの拡張です。編集用Markdownは各項・号・号の細分の平文末尾にHTMLコメントで文構造を記録します。コメント中の文をつないだ内容が表示用の平文と一致する場合に復元し、本文が編集された場合やコメントを取り除いた場合は、本文を一つのXMLの文として書き出します。この保持は、コメントが残る一行の本文に適用されます。


号や号の細分の本文が複数の `Column` で構成される場合は、列の順序・番号・改行・配置と、列に含まれる文を保持します。Lawtextでは文要素と同じ位置に `Column` 要素を記録し、編集用Markdownでは列の本文を空白でつないで表示します。列構造はHTMLコメントが残り、表示用の本文が一致する場合に復元します。


項見出しの `ParagraphCaption` と、識別用の `Num` 属性からは復元できない表示用の `ParagraphNum` も保持します。たとえば `Num="1"` に表示用の「１」がある場合、項の識別番号と表示用の「１」を別々に記録します。Lawtextでは項の文の前にXML要素として記録し、編集用Markdownでは一行の本文の末尾にあるHTMLコメントに含めます。Markdown上で本文を修正しても、コメント内の項見出しと表示番号を引き継ぎます。


法令名の読み `Kana`、略称 `Abbrev`、略称の読み `AbbrevKana` を保持します。ibisOfficeのLawtextでは題名の前に `:law-title-attrs: <LawTitle Kana="しけんほう" Abbrev=""/>` を置き、編集用Markdownではフロントマターの `law_title_attributes` に記録します。属性の空欄と省略は区別します。


項・号・号の細分の文に含まれるルビは、単純な `Ruby` と `Rt` の形をXMLへ書き戻します。Lawtextの文要素と編集用Markdownのコメントにも同じ形を記録し、参照・比較・改め文の平文にはルビの親文字を用います。


制定文 `EnactStatement` の文言は、Lawtextの `:law-enact-statement:` 行と編集用Markdownのフロントマターの `law_enact_statement` に保持します。

元のXMLに目次 `TOC` がある場合は、項目・順序・属性をそのXML断片として保持します。Lawtextでは `:law-toc:` 行、編集用Markdownではフロントマターの `law_toc` に記録します。本文の編章構成を編集した場合は、元の目次も確認してください。目次を本文から再生成する機能は別途必要です。

### 別表と様式の検証例

編集用Markdownでは、別表の見出しに `{#ibis_law_table_0}` のような識別子を付け、フロントマターにあるXML断片と対応付けます。別表を移動するときは見出しも一緒に移してください。識別子がない旧形式の文書は題名とセル構成で照合します。識別子を使用すると記録された文書では、識別子の削除や重複、参照先の欠落や破損を診断し、該当の元XML断片への書き戻しを行いません。

食品表示基準の別表第十八では、セルの縦結合と罫線を原XMLの別表断片として保持します。Lawtextと編集用Markdownで表題・関連条文・セルの構成が一致し、編集したセルが単一の平文の文であれば、元のXML断片の該当箇所だけを書き換えます。行やセルの数が変わった場合や、ルビや複数文を持つセルを編集した場合は、平文から表を再構成します。食品表示法に基づく検査の際に用いる様式では、`AppdxStyle` のXML断片を保持します。法令本文直下の図（`AppdxFig`）、注記（`AppdxNote`）、書式（`AppdxFormat`）、算式（`Appdx`）も同様に、Lawtextの `:law-appdx-fragments:` 行と編集用Markdownの `law_appdx_fragments` に記録します。様式名、書字方向、図の参照先（`Fig/@src`）が復元されます。参照先のPDFなどは元の相対パスに別途配置してください。別表とこれらの別添が混在する場合は、各別添より前にある別表の数をメタデータに保持し、法令XML出力で元の位置に戻します。本文直下の附則が別添の間にある場合も、相対順序を記録して復元します。旧形式の様式用メタデータも読み込めます。位置情報のない既存文書では別表の後に別添を出力します。いずれも往復後のXMLは公式XSDに適合しました。構造の欠落は診断JSONで確認できます。 [食品表示基準](https://laws.e-gov.go.jp/law/427M60000002010) [様式を定める命令](https://laws.e-gov.go.jp/law/427M60000042001/)

附則直下の別表（`SupplProvisionAppdxTable`）、様式（`SupplProvisionAppdxStyle`）、算式（`SupplProvisionAppdx`）は、その附則の見出し直後にXML断片を順番どおり保持します。Lawtextでは `:law-suppl-appendices:` 行、編集用Markdownでは同じ名前のHTMLコメントを使います。 Rustで附則を並べ替えると、見出しが同じ場合も本文で照合して別添を移します。同じ見出しを持つ附則の本文をRustで編集するときは、並べ替えた後の位置を `LawDoc::edit_suppl_provision` に渡すと、編集前に別添の所属を確定できます。見出しと本文が完全に同じ附則が複数ある場合は、同じ内容の附則の出現順で対応付けます。

別表を新設する改め文の検証例には、食品表示基準の令和六年内閣府令第七十一号による改正が使えます。e-Govの2024年4月1日版と9月1日版を比較すると、別表第二十六と第二十七が新設されています。結合セルのない2行・4行の別表で、その二表を抜粋した回帰テストで、追加位置・関連条文・全行の草案出力、逆適用、DOCXの表を検証しています。逆適用は現在保持している表題・関連条文・セルの平文を比較します。セルの結合・罫線や、府令全体の残差は引き続き確認が必要です。 [消費者庁の新旧対照条文](https://www.caa.go.jp/policies/policy/food_labeling/food_labeling_act/assets/food_labeling_cms201_240823_11.pdf)


号の末尾に細分を追加する草案には、その細分の下位項目も字下げして含めます。食品表示基準の令和六年内閣府令第七十一号による改正では、第二条と第二十二条の細分、および別表を逆適用すると、本則と別表の残差はなくなります。`--amendment-law-num '令和六年八月二三日内閣府令第七一号'` を指定すると、新設附則を法案附則の確認事項として表示します。 [消費者庁の新旧対照条文](https://www.caa.go.jp/policies/policy/food_labeling/food_labeling_act/assets/food_labeling_cms201_240823_11.pdf)
