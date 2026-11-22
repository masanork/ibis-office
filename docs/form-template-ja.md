# フォームテンプレートの書式と作り方

フォームを使うと、帳票のレイアウトを一度作り、個別データを差し替えて DOCX や PDF を生成できます。このガイドは試作の `docx_form` CLI を対象にしています。架空の指導要録では、氏名を1項目にまとめて様式1・2の4箇所へ反映し、学年別の所見や出欠も同じデータから出力します。[サンプルと実行手順](../test/fixtures/docx-form/full-school-record/README.md)

## 用意するファイル

| ファイル | 内容 |
| --- | --- |
| ひな型 | レイアウトと差し込み位置を持つ DOCX または HTML |
| フォーム定義 | フォーム ID、版、フィールド ID、値の型、必要に応じた字数上限、DOCX ひな型の SHA-256 |
| 個別データ | 参照先のフォーム ID と版、フィールドごとの値 |

フォーム定義と個別データにはそれぞれ `.json`、`.yaml`、`.yml` を使えます。たとえばフォーム定義を YAML で管理し、業務システムが JSON の個別データを出力できます。

## フォーム定義と個別データ

以下は進学先の2項目を持つフォーム定義の例です。DOCX を使う場合は `templateSha256` に実際のファイルの SHA-256 を入れます。HTML の合成ではこの項目は省略できます。

```yaml
form:
  id: school-transfer
  version: 1
templateSha256: "<ひな型 DOCX の SHA-256>"
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
