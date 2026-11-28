# GJS 異体字クラスタ 基本版 (SA 条件なし) — dict20261128

- 収録: 22,135 文字・8,463 クラスタ
- ファイル: `clusters.csv` (member_codepoint, cluster_id, representative_codepoint)、`variant-clusters.zst` (ibis-variant-bitmap/1+zstd)
- クラスタへの所属は「確認が必要な候補」を示すもので、同一人物や安全な置換を意味しない。
- 現代日本語では別字として扱う組 (托/拓・愉/愈・伙/火・䄷/石) には辺を張らない。
- 配布元: https://moji.sorane.dev/datasets/variant-clusters/
- 作成手順・評価: GJS リポジトリの `docs/legal/variant-cluster-data-sources.md`

## 出典と利用条件 (基本版)

| 出典 | 内容 | 条件 |
| --- | --- | --- |
| Unicode Unihan Database 17.0 (kSemanticVariant 等, kJinmeiyoKanji, kJoyoKanji, kJis0, kJIS0213, kMojiJoho) | 異体字関係、代表字の選定 | Unicode License V3 (LICENSE-Unicode.txt) |
| Mozc `src/data/single_kanji/variant_rule.txt` (「別字」区分は除外) | 旧字体・俗字・正字など | BSD-3-Clause, Copyright Google Inc. (LICENSE-Mozc.txt) |
| OpenCC `data/dictionary/JPShinjitaiCharacters.txt` | 常用・人名用・表外漢字字体表の新旧字体 | Apache License 2.0 (LICENSE-OpenCC.txt) |
| 国文学研究資料館 国書データベース異体字リスト (kokubunken/kokusho-itaiji-search) のうち他の資料と一致する組 | 異体字関係 | MIT, Copyright 2023 国文学研究資料館 (LICENSE-NIJL.txt) |
| 常用漢字表 (平成22年内閣告示第2号) の括弧内康熙字典体、人名用漢字、誤字俗字・正字一覧表など | 新旧字体・俗字 | 告示・法令・通達 (著作権法第13条) |
| デジタル庁「JIS X 0213への代替マップ 1.0版」のうちデジタル庁が独自に定義した区分1 | 行政事務標準文字 (GJ) と MJ 文字の代替 | 事務連絡 (通達に類するもの。著作権法第13条第2号) |
| GJS 追記分 (GJS リポジトリ `data/sources/custom/v2n_annotated_jp.csv` の出典 GJS の行) | 氏名で問題になるのに上記の資料に無かった組 | CC0 1.0 |

基本版は上記の各条件に従って再配布できる (SA 条件はない)。再配布時はこの NOTICE.md と LICENSE-*.txt を同梱すること。
