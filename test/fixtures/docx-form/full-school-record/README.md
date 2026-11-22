# 指導要録の帳票生成デモ

`form.yaml` と `record-b.yaml` は架空データのサンプルです。フォーム ID と版で対応を確認します。定義と個別データには JSON・YAML を混在させられます。`form.json` は字数制限を追加する前の定義です。

リポジトリ直下で実行します。

```sh
src=test/fixtures/docx-form/full-school-record
out=$(mktemp -d)
cargo run --locked -p ibis-formats --bin docx_form -- fill \
  "$src/template.docx" "$src/form.yaml" "$src/record-b.yaml" "$out/record.docx"
cargo run --locked -p ibis-office-app --bin ibisOffice -- \
  --export pdf --out-dir "$out" "$out/record.docx"
for layout in form-1-fixed form-2-flow; do
  cargo run --locked -p ibis-formats --bin docx_form -- fill-html \
    "$src/$layout.html" "$src/form.yaml" "$src/record-b.yaml" "$out/$layout.html"
done
```

- `maxChars` は表示形式を適用した後の書記素数です。改行も1文字に数え、上限を超えた末尾を省略記号なしで切り捨てます。入力ファイルの値はそのままです。
- `form-1-fixed.html` は A4 固定配置の試作です。`form-2-flow.html` は様式2の所見・出欠に絞り、行の高さが入力によって変わる印刷向けの試作です。ブラウザーで開いて A4・倍率100%で印刷できます。
- HTML ひな型は信頼できるファイルを使用し、差し込みマーカーは本文のテキストに置きます。固定配置の字数制限は、フォントによる実際の幅や高さの検査とは独立しています。

ひな型を作る手順と書式は [フォームテンプレートの書式と作り方](../../../../docs/form-template-ja.md) を参照してください。


DOCX ひな型はリッチ Markdown に変換して編集できます。`export-md` で書き出し、
フォーム定義に Markdown の `markdownTemplateSha256` を追加すると、
`check-md` と `fill-md` で直接使えます。回帰テストは、12表の列幅・行高・結合・
縦揃えと、372種類・計388箇所のフィールドを確認します。
