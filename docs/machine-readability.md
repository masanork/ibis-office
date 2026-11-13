# 機械可読性チェック

表計算の「ツール > 機械可読性チェック」では、編集中の結合セル・非表示行列・数式・機種依存文字や全角英数字を確認候補として表示します。文字の分類は Harunobu の L1-08 を参考にしていますが、シート全体の文字列を対象とし、表領域や文字コードによる合否は判定しません。表領域を推定せず、候補の合否や点数は出しません。

デスクトップ版は、別途インストールした Harunobu 0.2.2（コミット `88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f`）を使い、保存済み XLSX・XLSM・CSV・TSV の L1、L1–L2、L1–L3 を検査できます。検査時に保存済みファイルを一時ディレクトリにコピーし、完了後に削除します。AI 補完はオフで、結果は JSON または CSV で保存できます。スコアは未判定ルールを除いた Harunobu の参考値です。

## ローカル実行環境

macOS または Linux で Git と uv を用意し、ソースチェックアウトから次を実行します。

```sh
scripts/setup-harunobu.sh
```

固定コミットのソースと lockfile に基づく Python 環境を、macOS では `~/Library/Application Support/Sorane Docs/harunobu` に、Linux では `${XDG_DATA_HOME:-~/.local/share}/sorane-docs/harunobu` に作成します。アプリが検査を実行する時点でコミットと追跡ファイルの変更を確認します。ランタイムが無い場合や異なる場合は、検査を行わずエラーを表示します。

## 制限

- Harunobu は保存済みファイルを検査します。未保存の変更がある場合は保存してから実行してください。編集内容の候補表示は引き続き利用できます。
- 現在の Python 連携はデスクトップ版のみです。Windows 向けのセットアップと Web 版の Harunobu エンジンは未提供です。
- 複数ファイルを選択すると、ファイル横断の L3-07 と各ファイルの結果を検査できます。複数ファイル結果からセルへの直接移動はできません。
- AI 補完は未対応です。AI なしで判定できない項目は未判定と表示します。
- 表領域は Harunobu が推定します。複雑な表やタイトル・注釈では誤検出があり、公式の適合証明ではありません。詳しくは [Harunobu の既知の制限](https://github.com/digital-go-jp/machine_readability_rule/blob/88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f/docs/guides/limitations.md) を参照してください。
- 対応するセル位置は選択できます。非表示シートや未対応の位置表記は一覧に表示しますが、直接移動できません。

Harunobu のサンプル実装は MIT、ルールデータとドキュメントは PDL 1.0 です。出典: [digital-go-jp/machine_readability_rule](https://github.com/digital-go-jp/machine_readability_rule)。
