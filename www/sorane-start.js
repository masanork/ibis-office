// SPDX-License-Identifier: AGPL-3.0-only
// Configure the soraneDocs Web start view with the featured samples and landing panel.
(() => {
  if (new URLSearchParams(location.search).has('host')) return;

  const featuredItems = [
    {
      title: '機械可読ナレッジノート（Obsidian 互換）',
      subtitle: 'YAML・#tag・[[リンク]]・Mermaid を備えた Markdown 双方向 Split',
      name: 'sorane-knowledge-note.md',
      url: '/samples/sorane-knowledge-note.md',
      badge: 'MD',
    },
    {
      title: 'Marp スライド（双方向 Split）',
      subtitle: 'Markdown とスライド編集をリアルタイム同期・Mermaid 対応',
      name: 'marp-deck-sample.md',
      url: '/samples/marp-deck-sample.md',
      badge: 'MARP',
    },
    {
      title: 'Workbook Markdown（双方向 Split）',
      subtitle: 'Markdown テーブルと数式・複数シートをリアルタイム同期',
      name: 'workbook-sheet-sample.md',
      url: '/samples/workbook-sheet-sample.md',
      badge: 'SHEET',
    },
    {
      title: '架空市デジタル行政推進条例（Lawtext）',
      subtitle: '法令テキスト / e-Gov XML の双方向 Split とライブ法制執務 Lint',
      name: 'sample-ordinance.lawtext',
      url: '/samples/sample-ordinance.lawtext',
      badge: 'LAW',
    },
    {
      title: 'Mermaid 構文デモ集',
      subtitle: 'UML を含む主要 24 図種をまとめた Markdown 総集編',
      name: 'mermaid-showcase.md',
      url: '/samples/mermaid-showcase.md',
      badge: 'MD',
    },
    {
      title: 'AI 評価結果データ（JSONL）',
      subtitle: 'プロンプト・Eval スコア・Pass/Fail を構造化プレビューで検証',
      name: 'ai-eval-results-sample.jsonl',
      url: '/samples/ai-eval-results-sample.jsonl',
      badge: 'AI EVAL',
    },
    {
      title: 'ibisOffice WASM SBOM（CycloneDX 1.6）',
      subtitle: '本ビルド自身の依存部品目録 JSON を表計算として直接閲覧',
      name: 'ibis-office-wasm.sbom.cdx.json',
      url: '/samples/ibis-office-wasm.sbom.cdx.json',
      badge: 'SBOM',
    },
    {
      title: '名簿の照合（異体字・伏字名寄せ）',
      subtitle: 'M365 では扱いづらい異体字・伏字を含む名寄せデモ（架空データ）',
      name: 'roster-matching-sample.xlsx',
      url: '/samples/roster-matching-sample.xlsx',
      badge: 'XLSX',
    },
    {
      title: '自治体連携標準 CSV（構造化ビュー・294列）',
      subtitle: '20〜25連の繰り返し列をツリーで展開・検索（架空データ）',
      name: 'standard-linkage.csv',
      url: '/samples/standard-linkage.csv',
      badge: 'CSV',
    },
    {
      title: 'マイナンバー法改正案の新旧対照条文',
      subtitle: '縦書き条文と罫線表を含む一太郎文書（2023年3月1日版）',
      name: 'mynumber-law-2023-comparison.jtd',
      url: '/samples/mynumber-law-2023-comparison.jtd',
      badge: 'JTD',
    },
    {
      title: 'まちなかの居心地の良さを測る分析ツール',
      subtitle: '複数シートの数式と50個のグラフによる Excel 互換性デモ',
      name: 'urban-comfort-analysis.xlsx',
      url: '/samples/urban-comfort-analysis.xlsx',
      badge: 'XLSX',
    },
    {
      title: 'WordPerfect 6の表',
      subtitle: '結合セル・罫線・背景色を含む合成WPD',
      name: 'wordperfect-table.wpd',
      url: '/samples/wordperfect-table.wpd',
      badge: 'WPD',
    },
    {
      title: 'WordPerfect 6の埋め込み図',
      subtitle: '線形・放射状グラデーションと行内・浮動配置を含む合成WPD',
      name: 'wordperfect-graphics.wpd',
      url: '/samples/wordperfect-graphics.wpd',
      badge: 'WPD',
    },
    {
      title: 'スライドショー アニメーション・3D 切り替え検証',
      subtitle: 'グループ変形・トリガー・淡色テーマ色・3D 画面切り替え・ECMA-376 式/色/軌跡の検証デッキ（23スライド）',
      name: 'anim_stress_test.pptx',
      url: '/samples/anim_stress_test.pptx',
      badge: 'PPTX',
    },
  ];

  const startView = {
    listTitle: 'サンプル',
    items: [...featuredItems],
    selectAction: {
      label: 'GitHub でログイン',
      action: 'github-vault',
    },
    panel: {
      heading: 'Markdown と構造化ナレッジを、そのまま実務の文書へ。',
      body: [
        'Obsidian 互換の機械可読 Markdown（ノート・Marp スライド・Workbook 表計算・Lawtext/e-Gov 法令XML）を中心に、双方向 Split 編集と GitHub Vault 連携をブラウザ完結で使えるワークスペースです。',
      ],
      features: [
        {
          title: '全 4 形式の双方向 Source ↔ Visual Split 編集',
          description: 'Markdown ノート・Marp プレゼン・数式入り Workbook Markdown・Lawtext/e-Gov XML を左右リアルタイム同期で編集し、そのまま GitHub リポジトリへ保存できます。',
        },
        {
          title: 'AI・SBOM・一太郎・名寄せなど実務特化の可視化',
          description: 'AI 評価データ（JSONL）・CycloneDX SBOM・自治体連携標準 294 列 CSV・一太郎縦書き文書・異体字名寄せや高密度 Excel グラフまで端末内（WASM + WebGPU）で高速に処理します。',
        },
        {
          title: 'ブラウザ完結のナレッジ管理（GitHub Vault 連携）',
          description: 'GitHub Private リポジトリをそのまま Vault として接続し、ファイルツリー・全文検索・タグ・Backlinks・Local Graph を使って執筆や資料作成を進められます。',
        },
      ],
      links: [
        { label: '公式サイト', url: 'https://sorane.dev/' },
        { label: 'プライバシー', url: 'https://sorane.dev/privacy' },
        { label: 'ライセンス', url: './license.md' },
        { label: 'GitHub', url: 'https://github.com/sorane-dev/docs' },
      ],
      note: '形式や機能によっては元のアプリと表示・編集が異なる場合があります。大切な文書は原本を残してお使いください。',
    },
  };

  window.soraneStartBase = startView;
  window.ibisOfficeOptions = {
    ...(window.ibisOfficeOptions || {}),
    startView,
  };
})();
