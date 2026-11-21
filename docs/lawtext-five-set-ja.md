# 法令XML・Lawtextから法案五点セットを起案する

法律案の五点セットは、法律案の条文、理由、要綱、新旧対照表、参照条文で構成されます。Docsは、同一法令の現行版と改正後の溶け込み版から、五点の編集可能な草案を作ります。対象は一つの法令の一部改正です。複数法令を改正する法案は、対象法令ごとに差分を作り、法案全体の構成は担当者が組み立てます。[参議院「法令の改正方式（改め文、新旧対照表）」](https://www.sangiin.go.jp/japanese/annai/chousa/keizai_prism/backnumber/r03pdf/202120102.pdf)

## 入力を用意する

- **現行版**は、立案の基準日に適用する法令本文です。基準日と取得元を控えておきます。
- **改正後版**は、同じ法令に変更を反映した本文です。改め文を入力する場合は、別途、逆適用検算に渡します。
- **担当者メモ**には、制度上の課題、変更の狙い、根拠資料を記載します。
- **参照資料**は、外部法令の適用時点に対応する本文を必要に応じて用意します。

ファイルは UTF-8 の e-Gov 法令標準XML、Lawtext、または Docs の `type: lawtext` 付き Markdown を使えます。現行と改正後で異なる形式も使えます。法令番号が双方にある場合は表記もそろえます。

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

## 縦書きのセット版を DOCX と PDF にする

`--docx` は要綱、法律案、理由、新旧対照条文、参照条文をこの順に収録した一つの DOCX を出力します。A4縦判に右から左へ縦組みし、各文書は新しいページから始まります。新旧対照条文は、上段が改正案、下段が現行で、境界を黒罫で区切り、両段の変更箇所を傍線で示します。`--old-on-left` を付けると縦組みでは現行が上段になります。

組版の参考にした厚生労働省の公開例では、法律案は縦組みで、新旧対照条文は改正案と現行を上下に対照しています。公開例の要綱は横組みです。このセット版は五点を縦組みにそろえています。提出先に指定の様式があれば、編集用の DOCX で調整します。[法律案・理由](https://www.mhlw.go.jp/content/001686250.pdf) · [新旧対照条文](https://www.mhlw.go.jp/content/001686251.pdf) · [要綱](https://www.mhlw.go.jp/content/001686248.pdf) · [参照条文](https://www.mhlw.go.jp/content/001686254.pdf)

```sh
mkdir -p out
cargo run --locked -p ibis-formats --bin ibis-lawtext-5set -- \
  current.xml amended.lawtext --memo policy.md --reference cited-law.xml \
  --docx > out/bill-set.docx
cargo run --locked -p ibis-office-app --bin ibisOffice -- \
  --export pdf --out-dir out out/bill-set.docx
```

同じ DOCX から `out/bill-set.pdf` を生成します。Docs のデスクトップ版で DOCX を開き、編集後にPDFへ書き出すこともできます。指定書体は Noto Serif CJK JP です。PDFを作る環境ではこの書体を利用できるようにし、改ページ、傍線、罫線、文字のはみ出しを確認してください。lint と逆適用の診断一覧は上記の Markdown または JSON 出力で確認できます。

## Docs の AI Assistant で起案する

1. デスクトップ版の文書画面で改正後本文を開きます。
2. AI Assistant を開き、「現行法令 (.xml / .lawtext) を選んで 5点セット生成...」から現行版を指定します。
3. 目的、必要性、施行期日など判明している事項を指示に含めます。各草案を開く必要がある場合は、Markdown文書として開くよう依頼します。
4. 改め文を加筆した場合は、現行版と改正後版を使って再検算を依頼します。

AI Assistant の `generate_lawtext_5set` は五点の草案、構造化差分、検算結果、lint結果、起案用プロンプトを返します。外部法令本文を渡せる場合は `reference_laws` で参照条文に差し込めます。ローカル完結で処理する場合は上記のコマンドを使います。

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
