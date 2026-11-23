# フォームテンプレートの書式と作り方

フォームを使うと、帳票のレイアウトを一度作り、個別データを差し替えて DOCX、HTML、PDF を生成できます。このガイドは試作の `docx_form` CLI を対象にしています。架空の指導要録では、氏名を1項目にまとめて様式1・2の4箇所へ反映し、学年別の所見や出欠も同じデータから出力します。[サンプルと実行手順](../test/fixtures/docx-form/full-school-record/README.md)

## 用意するファイル

| ファイル | 内容 |
| --- | --- |
| ひな型 | レイアウトと差し込み位置を持つ DOCX、HTML、または AcroForm 付き PDF/A-2b |
| フォーム定義 | フォーム ID、版、フィールド ID、値の型、必要に応じた字数上限、DOCX または PDF のひな型の SHA-256 |
| 個別データ | 参照先のフォーム ID と版、フィールドごとの値 |

フォーム定義と個別データにはそれぞれ `.json`、`.yaml`、`.yml` を使えます。たとえばフォーム定義を YAML で管理し、業務システムが JSON の個別データを出力できます。

## フォーム定義と個別データ

以下は進学先の2項目を持つフォーム定義の例です。DOCX または PDF を使う場合は `templateSha256` に実際のファイルの SHA-256 を入れます。HTML の合成ではこの項目は省略できます。

```yaml
form:
  id: school-transfer
  version: 1
templateSha256: "<ひな型ファイルの SHA-256>"
fields:
  nextSchool.name:
    type: text
    maxChars: 28
  nextSchool.address:
    type: text
    maxChars: 36
```

個別データでは同じ `form` を指定し、値を `data` に置きます。入れ子のキーはドットでつないだフィールド ID に対応します。先頭ゼロに意味があるコードや日付などは文字列として引用符で囲みます。任意記入欄を空欄にする場合は、文字列フィールドに `""` を指定します。

```yaml
form:
  id: school-transfer
  version: 1
data:
  nextSchool:
    name: 架空市立みずき中学校
    address: 架空市みずき町３丁目
```

| 型 | 入力と表示 |
| --- | --- |
| `text` | 文字列。改行を含められます。 |
| `nonnegative-integer` | 0以上の整数。10進の数字で表示します。 |
| `date-ja` | `"2019-10-15"` のような日付文字列。平成・令和の範囲を和暦で表示します。 |

`maxChars` は任意の正の整数です。表示形式を適用した後で書記素を数え、超えた末尾を省略記号なしで切り捨てます。改行は1文字として数えます。変更されるのは出力上の表示だけです。日付や数値など、途中で切ると意味が変わる項目では、必要な桁数を確保してください。字数上限と、フォント・罫線・余白を含めた実際の欄への収まりは別々に検証します。

## DOCX ひな型を作る

1. 元の DOCX を複製し、表、結合セル、罫線、改ページを整えます。
2. 差し込み箇所にプレーンテキストのコンテンツコントロールを置き、タグを `ibis.form.nextSchool.name` のように設定します。進学先の学校名と所在地を別々に入れるには、同じ欄にタグを2つ置きます。同じ値を複数箇所に入れるときは同じタグを使います。
3. タグ付きのコントロールは本文の段落内に置きます。セルの中でも使用できます。仮の表示文字には短い文字列を使い、コントロール内は1個のテキストランにします。フィールド ID に使用できるのは ASCII 英数字、ドット、アンダースコア、ハイフンです。
4. `inspect` でフィールド ID と出現回数を確認し、定義の `fields` と個別データの `data` を揃えます。ひな型を保存したら、ファイルの SHA-256 を定義に記録します。ひな型を更新するときはフォーム版とハッシュを更新し、対応する個別データの参照も揃えます。

```sh
cargo run --locked -p ibis-formats --bin docx_form -- inspect template.docx
shasum -a 256 template.docx
cargo run --locked -p ibis-formats --bin docx_form -- \
  fill template.docx form.yaml record.yaml filled.docx
cargo run --locked -p ibis-office-app --bin ibisOffice -- \
  --export pdf --out-dir . filled.docx
```

生成 DOCX では差し込んだコントロールが通常の本文になります。署名付き DOCX はひな型として扱えません。

## Markdown で DOCX ひな型を編集する

DOCX のプレーンテキスト欄 `ibis.form.pupil.name` は、リッチ Markdown で
`{{form:pupil.name}}` になります。Markdown に同じマーカーを複数回書くと、
同じ値を各欄に差し込めます。マーカーを文字どおり表示するには先頭の
`{` をバックスラッシュでエスケープします。Markdown から生成した
DOCX にも既存のフォーム定義・個票データを使えます。

```sh
cargo run --locked -p ibis-formats --bin docx_form -- export-md template.docx template.md
cargo run --locked -p ibis-formats --bin docx_form -- inspect-md template.md
shasum -a 256 template.md
```

Markdown の SHA-256 をフォーム定義の `markdownTemplateSha256` に記録します。
DOCX 用の `templateSha256` と同じ定義に併記できます。Markdown のひな型を
更新したときはハッシュを更新します。

```sh
cargo run --locked -p ibis-formats --bin docx_form -- check-md template.md form.yaml record.yaml
cargo run --locked -p ibis-formats --bin docx_form -- fill-md template.md form.yaml record.yaml filled.docx
```

`check-md` はビルドと差し込みを検査し、字数の診断を返します。`fill-md` は
差し込み済み DOCX を保存します。Word で編集できる空欄付きの DOCX ひな型が
必要なら、`compile-md template.md rebuilt.docx` を使います。

Markdown の表を編集するときは、結合セルのマーカーと配置用の HTML コメントも
一緒に移動してください。`docx-table` は列幅（twip）、表の揃え、
セルの既定の縦揃え、先頭行の見出し指定を持ちます。`docx-row` は
行高（twip、`min` または `exact`）、`docx-anchor` はセルの縦揃えの
上書きです。`docx-p` はセル内段落の配置・間隔、`docx-size` は文字サイズ
（pt の100分の1）を保持します。本文の段落間隔は `docx-spacing` 属性で保持します。
HTML コメントは Markdown の通常の表示では隠れます。

Markdown は、Word で編集できる表をテキストで管理するのに向いています。
用紙上の座標で欄を指定したり、既存の紙様式に印字位置を合わせたりする場合は、
印刷用 CSS を使った HTML の固定配置が適しています。

## 印刷向け HTML ひな型を作る

HTML 本文のテキストに `{{nextSchool.name}}` のようなマーカーを置きます。信頼できるひな型を使い、マーカーは本文のテキストに置いてください。マーカーの参照先はフォーム定義の一部でも構いません。たとえば様式1と様式2の HTML を分け、両方で同じ個別データを使えます。

```html
<p class="school">{{nextSchool.name}}<br>{{nextSchool.address}}</p>
```

- 固定配置では、用紙サイズと欄の位置・幅・高さを mm で決めます。[様式1のひな型](../test/fixtures/docx-form/full-school-record/form-1-fixed.html)
- 行の高さが変わる帳票では、表の見出しを `thead` に置き、改ページ時に再表示します。所見など入力中の改行を表示する欄には `white-space: pre-wrap` を指定します。[様式2のひな型](../test/fixtures/docx-form/full-school-record/form-2-flow.html)

```sh
cargo run --locked -p ibis-formats --bin docx_form -- \
  fill-html layout.html form.yaml record.yaml filled.html
```

生成した HTML をブラウザーで開き、指定用紙・倍率100%で印刷または PDF 保存します。正式な出力条件を揃える場合はブラウザーとフォントを決め、通常値、字数上限に達する値、改行の多い値で、全ページを確認してください。

## 出力前に字数を確認する

`check` は DOCX、`check-html` は HTML のひな型に対して、出力時と同じ入力・ひな型の検証を行います。生成物のファイルは作成せず、切り詰めるフィールドの ID、表示形式適用後の字数、上限を JSON で標準出力に返します。同じフィールドが複数箇所にあっても結果は1件です。HTML では対象のページで使う欄について報告します。`fill` と `fill-html` も、切り詰めたフィールドを標準エラーに表示します。

```sh
src=test/fixtures/docx-form/enrollment-notice
cargo run --locked -p ibis-formats --bin docx_form -- \
  check-html "$src/layout.html" "$src/form.yaml" "$src/record.yaml"
```

この架空の入学予定通知書は、就学事務システムの標準レイアウトにある宛先、児童、保護者、予定校、特記事項を参考にしています。試験用に特記事項へ長い値を入れてあります。検査結果には氏名や住所などの値を含めません。物理的な欄の幅・高さ、フォント、印字項目の標準への適合は別に確認してください。 [文科省の標準レイアウト及び標準印字項目 第2.1版](https://www.mext.go.jp/content/20230405-mxt_syoto02-000024394_9.pdf)

デスクトップ版では、タグ付きの DOCX ひな型を開き、**校閲 → 指導要録の記載候補を確認…** からも検査できます。フォーム定義と個別データを選んで **字数を確認** を押し、結果を選ぶとひな型の該当欄に移動します。同じ欄が複数ある場合は本文の最初の箇所に移動します。

## PDF/A-2b の既存フォームに差し込む

未署名の PDF/A-2b にある AcroForm のテキスト欄を使用できます。`inspect-pdf` は完全修飾フィールド名と表示箇所数を返します。フォーム定義の `fields` に対応する名前（例 `student.name`）を指定し、`templateSha256` に PDF ひな型の SHA-256 を記録します。定義には対象の編集可能な標準テキスト欄をすべて記載します。値の型・表示変換と `maxChars` の扱いは DOCX・HTML と共通です。

```sh
cargo run --locked -p ibis-formats --bin docx_form -- inspect-pdf template.pdf
shasum -a 256 template.pdf
cargo run --locked -p ibis-formats --bin docx_form -- \
  check-pdf template.pdf form.yaml record.yaml /path/to/font.ttf /path/to/verapdf
cargo run --locked -p ibis-formats --bin docx_form -- \
  fill-pdf template.pdf form.yaml record.yaml /path/to/font.ttf /path/to/verapdf filled.pdf
```

両コマンドは veraPDF で入力と生成結果を PDF/A-2b として検証します。`check-pdf` はファイルを出力せず、`fill-pdf` は検証が通った場合に新規ファイルへ保存します。増分保存なので、ひな型の元の内容は出力内にも残ります。配布用のひな型には空欄か合成データを使用してください。

各ウィジェットの外観を、指定した埋め込み可能なフォントの先頭フェイスで再生成します。文字は黒の 11 pt で、フォームの左・中央・右揃え、グレー・RGB の背景と単純な実線枠を反映します。複数行欄では値に含まれる明示的な改行を使います。文字がフォントにない場合、PDF 側の `MaxLen` を超える場合、欄に収まらない場合はエラーになります。署名済み PDF、XFA、回転したウィジェット、リッチテキスト欄、チェックボックスや選択欄への入力は対象外です。

## 印刷用HTMLとVC向けのクレーム候補を確認する

印刷用HTMLのフォーム定義と個票に加え、クレーム名からフィールドIDへの対応を別のJSONまたはYAMLファイルに記述します。たとえば、研修修了証の対応表は次のようになります。

```yaml
form:
  id: training-completion
  version: 1
claims:
  training: course.title
  completion_date: completed
  number: serial
```

```sh
cargo run --locked -p ibis-formats --bin docx_form -- \
  preview-html-claims layout.html form.yaml record.yaml claims.yaml
```

出力は印刷用の `html`、未署名の候補を格納する `claims`、その他の帳票項目の切り詰めを示す `truncations` を含むJSONです。たとえば、紙面が「令和8年4月1日」でも `completion_date` は `"2026-04-01"` になります。管理番号 `"00123"` は文字列のままです。

対応表に指定した全項目について、印刷レイアウトへの差し込み、空欄の有無、字数上限による切り詰めを確認します。クレームが条件を満たさない場合はエラーになります。HTMLのレイアウトは、従来の帳票と同様に信頼できるひな型を使い、差し込み位置を印刷されるテキストに置いてください。実際の用紙サイズでの収まりは印刷プレビューで確認します。既存の帳票生成には対応表は不要です。

[架空の研修修了証で試す](../test/fixtures/docx-form/training-completion/README.md)と、正常な個票と切り詰めが起きる個票で、紙面の照合とクレーム候補の取り出しを確認できます。
