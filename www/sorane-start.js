// SPDX-License-Identifier: AGPL-3.0-only
// Configure the Docs Web start view with the featured samples and landing panel,
// and merge any additional public samples published via /api/samples.
(() => {
  if (new URLSearchParams(location.search).has('host')) return;

  const FEATURED_IDS = new Set(['bddbceda-c82b-4730-8744-a50e6ccb4600']);
  const featuredItems = [
    {
      title: 'Mermaid 構文デモ集',
      subtitle: 'UML を含む主要 24 図種をまとめた Markdown 総集編',
      name: 'mermaid-showcase.md',
      url: '/samples/mermaid-showcase.md',
      badge: 'MD',
    },
    {
      title: 'マイナンバー法改正案の新旧対照条文',
      subtitle: '縦書き条文と罫線表を含む一太郎文書（2023年3月1日版）',
      name: 'mynumber-law-2023-comparison.jtd',
      url: '/samples/mynumber-law-2023-comparison.jtd',
      badge: 'JTD',
    },
    {
      title: '表記揺らぎを含む名簿の照合',
      subtitle: '異体字や伏字を含む名寄せデモ（架空データ）',
      name: 'sample.xlsx',
      url: '/api/samples/bddbceda-c82b-4730-8744-a50e6ccb4600/sample.xlsx',
      badge: 'XLSX',
    },
    {
      title: 'まちなかの居心地の良さを測る分析ツール',
      subtitle: '複数シートの数式と50個のグラフを含む記入例',
      name: 'urban-comfort-analysis.xlsx',
      url: '/samples/urban-comfort-analysis.xlsx',
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
      title: 'Visio マルチページ図面ハンドブック',
      subtitle: '複数ページのフロー図・構成図を含む Visio 図面（合成データ）',
      name: 'visio-multi-page-handbook.vsdx',
      url: '/samples/visio-multi-page-handbook.vsdx',
      badge: 'VSDX',
    },
    {
      title: 'soraneDocs WASM SBOM（CycloneDX 1.6）',
      subtitle: '本ビルド自身の依存部品目録 JSON を表計算として直接閲覧',
      name: 'ibis-office-wasm.sbom.cdx.json',
      url: '/samples/ibis-office-wasm.sbom.cdx.json',
      badge: 'SBOM',
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
      heading: '文書をひらく。ひらめきが、広がる。',
      body: [
        'ブラウザだけでさまざまな文書をひらいて、読んで、編集できるオフィスアプリです。左のサンプルはページを切り替えずにそのまま開け、閉じるとこの画面に戻ります。',
      ],
      features: [
        {
          title: '幅広い文書形式をブラウザで直接表示',
          description: 'Word・Excel・PowerPoint に加え、一太郎・DocuWorks・Visio・Markdown (Mermaid)・自治体連携標準 CSV に対応しています。',
        },
        {
          title: '端末内完結（WebAssembly + WebGPU）',
          description: '文書の読み込み・編集・PDF 書き出しはすべてブラウザ内で処理されます。ローカルのファイルも「開く…」からそのまま試せます。',
        },
        {
          title: '日本語組版・クラウド保存・AI アシスタント',
          description: '縦書き・罫線表・異体字対応フォントを備え、ログインすると個人ファイルの保存や Git リポジトリ連携、AI アシスタントも利用できます。',
        },
      ],
      links: [
        { label: '保存したファイル', url: '/files' },
        { label: 'リポジトリ', url: '/repos' },
        { label: 'ライセンス', url: './license.md' },
        { label: 'GitHub', url: 'https://github.com/masanork/ibis-office' },
      ],
      note: '形式や機能によっては元のアプリと表示・編集が異なる場合があります。大切な文書は原本を残してお使いください。',
    },
  };

  window.soraneStartBase = startView;
  window.ibisOfficeOptions = {
    ...(window.ibisOfficeOptions || {}),
    startView,
  };

  void (async () => {
    try {
      const response = await fetch('/api/samples', { headers: { Accept: 'application/json' } });
      if (!response.ok) return;
      const samples = await response.json();
      if (!Array.isArray(samples)) return;
      let added = false;
      for (const sample of samples.slice(0, 100)) {
        if (
          typeof sample.id !== 'string' || !/^[a-f0-9-]{36}$/.test(sample.id) ||
          typeof sample.name !== 'string' || !/^[a-zA-Z0-9_-]{1,80}\.(?:docx|xlsx|pptx)$/.test(sample.name) ||
          typeof sample.title !== 'string' || sample.title.length > 100 ||
          FEATURED_IDS.has(sample.id)
        ) {
          continue;
        }
        const ext = sample.name.split('.').pop().toUpperCase();
        startView.items.push({
          title: sample.title,
          subtitle: sample.name,
          name: sample.name,
          url: `/api/samples/${sample.id}/${sample.name}`,
          badge: ext,
        });
        added = true;
      }
      if (added && typeof window.ibisOfficeSetStartView === 'function') {
        window.ibisOfficeSetStartView(startView);
      }
    } catch {
      /* Bundled samples remain available even when /api/samples is unconfigured. */
    }
  })();
})();
