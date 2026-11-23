# 研修修了証のレビュー試用

所要時間は5〜10分です。CLIでレビュー用HTMLを生成し、ブラウザーで印刷用の証明書と発行システムに渡すクレーム候補を確認します。個票は2件とも架空です。

## 参加者の課題

1. 正常な個票で、印刷用HTMLをブラウザーで開き、クレーム候補と見比べてください。修了日と管理番号が照合できたら、引き渡すJSONを示してください。
2. 次の個票で出力を試し、どの欄を見直せばよいか、表示された情報から判断してください。修正が必要な場合は、どこに確認を戻すかも説明してください。

### 操作手順

リポジトリのルートで実行します。出力先はシステムの一時ディレクトリです。

```sh
fixture=test/fixtures/docx-form/training-completion
review_dir="$(mktemp -d)"

cargo run --locked -q -p ibis-formats --bin docx_form -- \
  review-html-claims "$fixture/layout.html" "$fixture/form.yaml" \
  "$fixture/record.yaml" "$fixture/claims.yaml" "$review_dir/review.html"
printf '%s\n' "$review_dir/review.html"
```

表示された `review.html` をブラウザーで開きます。紙面と照合してから、「クレームをJSONで保存」で引き渡し用のファイルを保存します。

```sh
cargo run --locked -q -p ibis-formats --bin docx_form -- \
  preview-html-claims "$fixture/layout.html" "$fixture/form.yaml" \
  "$fixture/record-overlong.yaml" "$fixture/claims.yaml"
```

エラーには対応表の欄名とフィールドID、実際の字数と上限が表示されます。帳票にある他の欄も確認するには、同じ個票を帳票チェックにかけます。

```sh
cargo run --locked -q -p ibis-formats --bin docx_form -- \
  check-html "$fixture/layout.html" "$fixture/form.yaml" "$fixture/record-overlong.yaml"
```

## 進行役の確認項目

試用中は下の判定を参加者に先に伝えず、迷った操作と判断に使った表示を記録します。

| 課題 | 判断の目安 |
| --- | --- |
| 日付の照合 | 印刷用HTMLの「令和8年4月1日」と `completion_date` の `2026-04-01` を同じ日と判断できる。 |
| 管理番号と引き渡し | `number` が文字列 `"00123"` で、引き渡すファイルが `claims.json` だと分かる。 |
| 切り詰め時の対応 | エラーの「管理番号 (serial)」から紙面の欄を見つけ、上限5文字に対して6文字あると判断できる。番号を独断で短くせず、個票の原本か帳票の字数上限を確認する。 |

紙面とクレームの照合で迷った箇所、引き渡すファイルの見分け方、エラーの欄名と字数から次に確認すべきことを判断できたかを記録します。
