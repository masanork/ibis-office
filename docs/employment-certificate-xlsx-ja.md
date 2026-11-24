# 就労証明書のXLSX受け渡しと項目対応

こども家庭庁が2026年8月5日に掲載資料を更新した標準XLSXの「標準的な様式」シートを対象にする。セルの位置はこの版に対応する。別の版に適用する際は、欄の見出しと位置を照合する。結合セルは左上のセルを参照する。 [こども家庭庁の標準様式](https://www.cfa.go.jp/policies/hoiku/shugyoshomeisho)

## 受領側で確認すること

ibisOfficeで記入・保存した固定勤務と変則勤務の架空サンプルを使う。

1. Microsoft Excelで開き、修復を求められないことと、本人氏名・証明日・勤務時間が入力どおりであることを確認する。
2. 雇用形態（正社員の `I23`、パート・アルバイトの `L23`）を選び、別シートを参照するプルダウンに `□` と `☑` が表示されることを確認する。
3. 「標準的な様式」シートをアクティブにして印刷プレビューを開き、A4縦1ページに帳票本体が収まり、チェック印と記入内容が読めることを確認する。

Gnumeric 1.12.57では公式原本と保存済みの2件を読み込めた。保存済みの本人氏名、雇用形態のチェック、勤務時間と、別シート参照の入力規則を確認した。帳票本体はA4縦の先頭1ページに収まった。ブック全体のPDFには補助シートも出力される。

Gnumericの印刷結果では、元の様式でも項目番号が指数表記になり、記入済みファイルでは狭いセルの雇用開始年や固定勤務の月間時間にも指数表記が見られた。該当セルのXLSXの表示形式は原本と保存済みファイルで同じだった。Microsoft Excelでの修復通知、プルダウンの操作、印刷表示は未確認。

## 会社が証明する項目の対応案

受け渡し候補の最小範囲。項目名とデータ形式は接続先と確認する。

| 項目 | 標準様式のセル | データの扱い |
| --- | --- | --- |
| 証明事業所名 | `Z4` | 文字列。No.4の本人就労先事業所とは別項目。 |
| 本人氏名 | `I19` | 文字列。 |
| 証明日 | `AA3`・`AF3`・`AI3` | 年・月・日から実在する西暦日付を組み立てる。 |
| 雇用形態 | No.5、サンプルでは `I23`・`L23` | `☑` をそれぞれ「正社員」「パート・アルバイト」と対応付ける。他の選択肢は追加のマッピングで扱う。 |
| 固定勤務の合計 | `W25`・`AA25`・`AG25` | 月間の時間・分・うち休憩分。 |
| 固定勤務の就労日数 | `Q27`・`AC27` | 月間と週間の就労日数を、それぞれの単位付きで保持する。 |
| 変則勤務の合計 | `M31`・`P31`、`S31`・`W31`・`AC31` | チェック欄から月間／週間を読み、時間・分・うち休憩分と組み合わせる。 |
| 変則勤務の就労日数 | `M32`・`P32`、`S32` | 合計時間の単位とは別に、日数の月間／週間を読む。 |

空欄とゼロは区別する。勤務時間の固定／変則が曖昧な記入や、変則の単位が未選択・両方選択の記入は、人が確認する項目とする。No.19「保護者記載欄」の児童名・生年月日・施設名は、保護者が申請時に記載する情報として、会社の受け渡し候補から除く。

保存済みサンプルから読み取った対応は次のとおり。変則勤務のサンプルは時間と日数の両方で月間を選択している。

| サンプル | 雇用形態 | 合計時間 | 就労日数 |
| --- | --- | --- | --- |
| 固定勤務 | 正社員 | 月間180時間0分、うち休憩1,200分 | 月間20日、週間5日 |
| 変則勤務 | パート・アルバイト | 月間96時間30分、うち休憩480分 | 月間16日 |

## 就労データのスキーマ試作

[版付き対応表](../scripts/employment-schema/mapping.json)から、就労データ部分の [JSON Schema](../scripts/employment-schema/employment.schema.json) と抽出結果を生成する。試作の対象は正社員・パート／アルバイトの2区分と、上表の勤務時間・日数。対応表を使う前に、対象の様式版とシートを確認する。

```sh
python3 scripts/employment-schema/pilot.py schema
python3 scripts/employment-schema/pilot.py extract cells.json
python3 scripts/employment-schema/test_pilot.py
```

`cells.json` は対象シートのA1セル番地をキー、セル値を値とするJSONオブジェクト。ibisOfficeの `xlsx_get_cells` の値をセル番地に対応付けて渡せる。試作の検証にはPythonの `jsonschema` を使う。試作では対応表にある項目の完全な入力を求め、未対応の雇用形態や曖昧な勤務区分をエラーにする。この必須条件は試作用で、提出要件は受領側と確認する。

保存済みの固定・変則勤務から抽出したデータはスキーマ検証を通過した。変則勤務で合計時間を月間、日数を週間にした値でも検証し、保護者欄に架空の児童名・施設名があっても結果が変わらないことを確認した。

## LLMによる候補作成の評価方法

空の公式XLSXから取得した見出し・結合範囲・入力規則と記載要領を使う。LLMには、項目名、セル番地、値の型、単位、記載主体、根拠を候補として出させる。確定したマッピングは入力から外し、出力後に比較する。確認するのは次の四点。

- 証明事業所と本人就労先事業所を区別したか。
- 証明日を三つのセルから組み立てる提案になっているか。
- 変則勤務の時間と日数を別々の単位で扱っているか。
- 保護者記載欄を会社の証明対象から除き、根拠が足りない必須条件を確認事項にしたか。

候補のセル番地は実際のXLSXと機械的に照合し、意味と必須条件は担当者が承認する。対応表を見ていないモデルでの再現率測定は今後の評価とする。

## 候補をレビューして対応表を確定する

[候補サンプル](../scripts/employment-schema/candidate.example.json)は、LLM出力を受け取る形式を試すために作成した。変則勤務の日数に合計時間のチェック欄を割り当てる誤りを入れてある。実際の推論精度を示すサンプルではない。

```sh
tools=scripts/employment-schema
review_dir="$(mktemp -d)"
cp "$tools/candidate.example.json" "$review_dir/review.json"
python3 "$tools/review.py" show "$review_dir/review.json"
```

担当者は候補と根拠を公式様式で照合し、`review.json` の `mapping` を修正する。各項目群の `review_note` に判断や修正理由を書き、確認が済んだ群の `decision` を `accepted` にする。別のモデルで試す場合は、空の様式と記載要領に加え、項目と根拠の出力形式を指示する。正解のセル番地を含む候補サンプルはレビュー側で使う。

```sh
python3 "$tools/review.py" approve "$review_dir/review.json" \
  --reviewer '確認担当者名' > "$review_dir/approved.json" &&
python3 "$tools/pilot.py" schema --approved-mapping "$review_dir/approved.json"
```

承認済み対応表を抽出にも使う場合は、`pilot.py extract cells.json --approved-mapping "$review_dir/approved.json"` を実行する。承認待ちの項目群、重複するセル、会社記載範囲外のセル、承認後に変更された対応表を検出すると処理を止める。承認ファイルは担当者名と判断の記録であり、本人確認や電子署名は別の仕組みで扱う。

## OKF原稿から承認済みの対応表を使う

[就労証明書のOKF原稿例](../scripts/employment-schema/employment.example.md)を、前節で作成した `approved.json` と同じディレクトリに置く。PythonにPyYAMLが必要。

```sh
cp "$tools/employment.example.md" "$review_dir/employment.md"
python3 "$tools/pilot.py" schema --okf "$review_dir/employment.md"
python3 "$tools/pilot.py" extract cells.json --okf "$review_dir/employment.md"
```

試作仕様では、フロントマターの `type` を `credential-definition` とし、拡張キー `x-ibis-credential` に整数の `version: 1` と `approved_mapping` を書く。この版は拡張キーの読み方を示す。対応表自体の版は承認ファイルで管理する。Markdown本文は説明と根拠の参照に使う。

参照先は原稿のディレクトリを起点とするローカルの相対JSONパス。サブディレクトリも使える。親ディレクトリ参照、原稿のディレクトリ外に出るシンボリックリンク、URL、重複するYAMLキーはエラーにする。承認の有無と対応表の変更は既存の承認検査で確認する。

固定勤務・変則勤務のスキーマや抽出結果は、承認済みJSONを直接指定した場合と同じになる。提示要求で使う論理項目ID、SD-JWT VCとmdocへの割り当て、DCQLの生成規則は、提示要求を試作する段階で検討する。

## 保育利用申込の提示要求をDCQLに変換する

[提示要求のOKF原稿例](../scripts/employment-schema/childcare-request.example.md)を、`employment.md` と同じディレクトリに置く。原稿の `type` は `presentation-request`。拡張キー `x-ibis-presentation-request` に版、資格情報原稿への相対パス、用途、要求する論理項目IDを記述する。

```sh
cp "$tools/childcare-request.example.md" "$review_dir/childcare-request.md"
python3 "$tools/presentation.py" "$review_dir/childcare-request.md"
```

資格情報原稿の `sd_jwt_vc` に試作用の `vct` と、各IDのクレームパスを明示する。今回の対象は固定・変則の両方にある項目。論理IDは現行の就労データのプロパティ階層をドットでつなぎ、SD-JWT VC用のクレームパスは同じ階層を配列で指定する。これにより、承認済み対応表から生成したJSONと照合できる。将来、形式ごとに別の階層を使う場合は、変換規則をその形式の対応に記述する。

`work_schedule.total` は単位・時間・分・うち休憩分を含むオブジェクトとして要求する。実際の交付では同じ単位で開示できるようにエンコードする必要がある。出力は保有者の暗号学的な束縛を要求するDCQL。用途は原稿に記録し、検証者IDと `nonce` は実際の申請時に検証側で設定する。 [OpenID4VP 1.0](https://openid.net/specs/openid-4-verifiable-presentations-1_0-final.html)

同じ提示要求をmdoc用に変換するには、`presentation.py` に `--format mso_mdoc` を指定する。省略時はSD-JWT VCを出力する。

```sh
python3 "$tools/presentation.py" "$review_dir/childcare-request.md" --format mso_mdoc
```

資格情報原稿の `mdoc.doctype` は試作の文書タイプを表す。`mdoc.paths` は論理項目IDを `[名前空間, 要素名]` に対応付ける。`certification_date` は西暦日付の文字列、`work_schedule_total` は単位・時間・分・うち休憩分を含む一つのCBORマップを想定する。合計の一部分だけを要求するには、交付側で要素の設計を変える必要がある。

提示要求の任意の `intent_to_retain` はmdocの各要求要素に反映する。例では `false` を指定している。これは試作用の要求における受け手の保持の意図を表す。実際の保育申込での保持方針は提出先が決める。試作で確認するのはDCQLの生成までで、CBORデータの発行・Walletへの交付・提示検証は実装範囲外。 [OpenID4VP 1.0のmdoc形式](https://openid.net/specs/openid-4-verifiable-presentations-1_0-final.html#appendix-B.2)

## テスト鍵で開示内容を照合する

```sh
cargo build --locked -p ibis-doc-service --example employment_presentation
python3 scripts/employment-schema/test_presentation_roundtrip.py target/debug/examples/employment_presentation
```

架空の変則勤務データ一件から、SD-JWT VCとmdocの試験用データを作る。OKF原稿から生成したDCQLを提示比較器に直接渡し、実際の開示項目と照合する。雇用形態と就労日数は発行時のデータに含め、提示時には開示しない。

- SD-JWT VCでは `work_schedule.total` をネストした一つの開示にし、単位・時間・分・休憩分を同時に返す。合計オブジェクトの内部でさらに選択的開示を使う場合は、この試作で確認した開示粒度と異なる。
- mdocでは同じ合計を名前空間内の一つの要素にし、値をCBORマップにする。単位はテキスト、時間・分・休憩分は整数で、証明日は西暦日付のテキストとして扱う。
- SD-JWT VCでは、同じ提示に対して異なる宛先や `nonce` を指定すると不一致を検出する。mdocの既存診断では発行者署名と開示ダイジェストを確認し、端末認証と申請への束縛の判定は未確認として扱う。

提示比較器は、証明日と合計だけの提示を過不足なしと判定し、合計がない提示と、雇用形態・就労日数も出した提示をそれぞれ不足・余分として報告する。SD-JWT VCではネストしたパスと、名前そのものにドットを含む項目を区別する。実Walletと検証者の間でのOpenID4VPセッションは別途検証する。
