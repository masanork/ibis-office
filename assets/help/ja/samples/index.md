---
type: help
title: サンプル・レンダリング・ウォークスルー
description: 法令（番号利用法）をはじめ各種機械可読フォーマットの実際のレンダリングを体験するガイド
tags: [help, samples, walkthrough]
help_version: 1
profile: sorane-okf/0.1
---

# サンプル・レンダリング・ウォークスルー

[[help/index|← ヘルプ目次に戻る]]

`help/samples/` フォルダには、**実際の法令データ（番号利用法）や各種機械可読フォーマット**のサンプルファイル一式が収録されています。
下のリンクから各ファイルを順番に開き、**[Source | Split | Visual]** 切替で **Split** を選ぶと、元データとレンダリング結果を並べてウォークスルーできます。

---

## Step 1. 法令（番号利用法：平成二十五年法律第二十七号）

e-Gov 法令 API から取得した **行政手続における特定の個人を識別するための番号の利用等に関する法律（番号利用法 / マイナンバー法）** 第1章（総則）の実データです。

1. **e-Gov 法令標準 XML**: [[help/samples/mynumber-act-ch1.xml|mynumber-act-ch1.xml]]
   * **レンダリングの見どころ**: `<Law>` / `<Article>` / `<Paragraph>` タグが自動的に日本の法令書式（条・項・号・見出し括弧）へ整形され、右ペインに構造化プレビューが表示されます。左ペインの XML を編集すると対象文のみが安全に更新されます。
2. **Lawtext（人間可読テキスト形式）**: [[help/samples/mynumber-act-ch1.lawtext|mynumber-act-ch1.lawtext]]
   * **レンダリングの見どころ**: XML と同じ内容がインデントベースの読みやすい **Lawtext** として表示され、法制執務 Lint や条文構造のプレビューと双方向同期します。

---

## Step 2. Markdown 系の構造化ドキュメント

3. **OKF 機械可読ナレッジノート**: [[help/samples/knowledge-note.md|knowledge-note.md]]
   * **レンダリングの見どころ**: YAML フロントマター、`#tag`、`[[wikilink]]`、Callout（`[!TIP]`）、タスクリスト、表、数式、脚注がリッチ文書として描画されます。
4. **Marp スライド**: [[help/samples/marp-deck.md|marp-deck.md]]
   * **レンダリングの見どころ**: `marp: true` と `---` により、左の Markdown が右ペインで **スライドデッキ** として描画され、スライド内の Mermaid 図もそのままレンダリングされます。
5. **Workbook Markdown（表計算）**: [[help/samples/workbook-sheet.md|workbook-sheet.md]]
   * **レンダリングの見どころ**: `ibis-workbook: true` とテーブル記法から **複数シートのワークブック** が構築され、`=SUM(B2:B3)` などの数式がリアルタイムに再計算されます。
6. **Mermaid 図表集**: [[help/samples/mermaid-diagrams.md|mermaid-diagrams.md]]
   * **レンダリングの見どころ**: `flowchart`・`sequenceDiagram`・`pie`・`xychart-beta` がベクター図表・グラフとして描画されます。

---

## Step 3. 機械可読データ・オープンデータ標準

7. **CycloneDX 1.6 CBOM / SBOM（JSON）**: [[help/samples/payment-demo.cdx.json|payment-demo.cdx.json]]
   * **レンダリングの見どころ**: ソフトウェア部品表・暗号資産目録（CycloneDX JSON）が構造化テーブルとして閲覧できます。
8. **AI 評価結果ログ（JSONL）**: [[help/samples/ai-eval-results.jsonl|ai-eval-results.jsonl]]
   * **レンダリングの見どころ**: 1行1JSON の評価レコードが、プロンプト・期待値・スコア・Pass/Fail 付きの構造化テーブルとして表示されます。
9. **FDSN StationXML（地震観測点 XML）**: [[help/samples/stations.xml|stations.xml]]
   * **レンダリングの見どころ**: 観測網・観測点・チャネルの階層を持つ標準 XML が構造化プレビューとして整理表示されます。
10. **名寄せ候補データ（CSV）**: [[help/samples/field-match-candidates.csv|field-match-candidates.csv]]
    * **レンダリングの見どころ**: 日本語氏名・住所・生年月日を含む CSV が表計算グリッドとして開かれ、ソートや照合・集計をそのまま試せます。
