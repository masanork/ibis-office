# Markdown の Mermaid 図

Markdown の `mermaid` コードブロックを文書やスライドに読み込むと、対応する図は表示用の SVG または Office グラフに変換されます。対応は Mermaid の構文すべてを網羅しているわけではありません。大切な図は、読み込み後と保存・再読込後に内容を確認してください。

- Flowchart、sequence、ER、state、Gantt などは SVG 図として扱います。図そのものは Office の図形として個々に編集できません。
- Pie と xychart は通常、編集できる Office グラフになります。`xychart horizontal` の棒グラフは編集できる横棒グラフになり、折れ線を含む場合は SVG になります。数値範囲で x 軸を指定した図も SVG になります。貼り付けなど、取り込み状況によっては SVG になります。グラフを編集した場合、古い Mermaid ソースをそのまま書き戻す代わりに `chart` ブロックとして出力することがあります。
- 対応する図は Markdown に書き出す際、保存された Mermaid ソースを使います。未対応の図や解釈できない図は、`mermaid` の言語指定を含む編集可能なコードとして表示します。DOCX や PPTX に保存して再読込した後も、Markdown に書き出すときにそのコードブロックを復元します。
- 対応済みの図種でも、一部の構文や設定が描画されないことがあります。例えば flowchart の画像ノードは画像の代わりに枠を表示し、外部画像を取得しません。リンクやアニメーションなど、静的な文書では動作しない機能もあります。
- 固定した Mermaid 12.3.0 の公式例539件を描画対象としています。アイコンパックが組み込まれていない図では代替表示になることがあります。

[図種別デモ](mermaid-demo-ja.md) は Docs で直接開けます。公式例をすべて試す手順は [Markdown Reference](markdown-reference-ja.md) にあります。

テスト対象と既知の構文差は [Mermaid compatibility corpus](../crates/ibis-formats/tests/mermaid/README.md) と [features.txt](../crates/ibis-formats/tests/mermaid/features.txt) に記載しています。コーパスで描画できることや参照実装とのラベル一致は、図の意味やレイアウトの完全な一致を保証しません。

PlantUML コードブロックのネイティブ IR 描画（18 図種対応）および互換性・変換境界の詳細は [PlantUML 対応と変換境界](plantuml-sequence-conversion.md) を参照してください。
