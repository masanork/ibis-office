// SPDX-License-Identifier: AGPL-3.0-only
// Configure the soraneDocs Web start view with the featured demos and landing panel.
(() => {
  if (new URLSearchParams(location.search).has('host')) return;

  const featuredItems = [
    {
      title: 'Markdown → DOCX 技術設計書',
      subtitle: 'Mermaid 図・表・チェックリストを含む Markdown からそのまま美しい Word 文書を生成',
      name: 'markdown-to-docx-proposal.md',
      url: '/samples/markdown-to-docx-proposal.md',
      badge: 'MD → DOCX',
    },
    {
      title: 'Markdown → PPTX プレゼン設計',
      subtitle: 'Marp フロントマターと双方向 Split 編集から PowerPoint スライドを生成',
      name: 'markdown-to-pptx-deck.md',
      url: '/samples/markdown-to-pptx-deck.md',
      badge: 'MD → PPTX',
    },
    {
      title: '番号利用法（マイナンバー法・抄）',
      subtitle: '平成二十五年法律第二十七号（Lawtext / e-Gov XML 双方向編集・改正案 JTD と対比可能）',
      name: 'mynumber-law.lawtext',
      url: '/samples/mynumber-law.lawtext',
      badge: 'LAW',
    },
    {
      title: 'マイナンバー法改正案の新旧対照条文',
      subtitle: '縦書き条文と罫線表を含む一太郎文書（2023年3月1日版）',
      name: 'mynumber-law-2023-comparison.jtd',
      url: '/samples/mynumber-law-2023-comparison.jtd',
      badge: 'JTD',
    },
    {
      title: 'LLM 評価ベンチマーク結果（JSONL）',
      subtitle: 'Pass/Fail・スコア・レイテンシ・期待値比較を構造化ビューで可視化',
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
      title: 'Mermaid 構文デモ集',
      subtitle: 'UML を含む主要 24 図種をネイティブ描画する Markdown 総集編',
      name: 'mermaid-showcase.md',
      url: '/samples/mermaid-showcase.md',
      badge: 'MERMAID',
    },
    {
      title: '自治体連携標準 CSV（構造化ビュー・294列）',
      subtitle: '20〜25連の繰り返し列をツリーで展開・検索',
      name: 'standard-linkage.csv',
      url: '/samples/standard-linkage.csv',
      badge: 'CSV',
    },
    {
      title: 'まちなかの居心地の良さを測る分析ツール',
      subtitle: '複数シートの数式と50個のグラフによる Excel 互換性デモ',
      name: 'urban-comfort-analysis.xlsx',
      url: '/samples/urban-comfort-analysis.xlsx',
      badge: 'XLSX',
    },
    {
      title: 'スライドショー アニメーション・3D 切り替え検証',
      subtitle: 'グループ変形・トリガー・3D 画面切り替え・ECMA-376 式の検証デッキ（23スライド）',
      name: 'anim_stress_test.pptx',
      url: '/samples/anim_stress_test.pptx',
      badge: 'PPTX',
    },
  ];

  const startView = {
    listTitle: 'デモ・実務データ',
    items: [...featuredItems],
    selectAction: {
      label: 'GitHub でログイン',
      action: 'github-vault',
    },
    panel: {
      heading: 'ひとつのテキストから、美しい文書・スライド・表計算・法令へ。',
      body: [
        '新規作成の「text」またはお手元のテキストファイルを開くと、先頭のタグや書式（marp: true / ibis-workbook: true / Lawtext / e-Gov XML）を自動判定して最適な編集ビューへ動的に切り替わります。',
      ],
      features: [
        {
          title: 'Markdown からそのまま美しい DOCX・PPTX を生成',
          description: 'Mermaid 図や表を含むテキストを左右リアルタイム同期で執筆し、ワンクリックで配布品質の Word（.docx）・PowerPoint（.pptx）として出力できます。',
        },
        {
          title: 'LLM 評価データ・SBOM・法令・一太郎の構造化表示',
          description: 'AI 評価結果（JSONL）・CycloneDX SBOM・番号利用法（Lawtext/e-Gov XML）・一太郎（JTD）新旧対照条文・294 列 CSV まで端末内（WASM + WebGPU）で高速に可視化します。',
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
