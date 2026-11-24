# 気象庁防災情報XMLの共有表示定義

[気象庁の技術資料](https://xml.kishou.go.jp/tec_material.html)に掲載された
[2026年11月18日版サンプルZIP](https://xml.kishou.go.jp/jmaxml_20261118_Samples.zip)から
次のXMLを取り出します。

| 公開サンプル | 適用される表示定義 | 表示する内容 |
| --- | --- | --- |
| `02_01_01_201224_VGSK55.xml` | `phenology.view.json` | さくらの開花日、和歌山地方気象台 |
| `03_01_01_091210_VGSK60.xml` | `special-weather.view.json` | 黄砂、横浜地方気象台、視程の補足 |

デスクトップ版の **設定 → 一般 → 表示定義フォルダーを開く** から
2件の `*.view.json` を共通フォルダーにコピーします。XMLだけを別のフォルダーに
置いて開くと、同じ名前空間とルート要素 `Report` を使う電文が、
`Head` の `InfoKind` と `InfoKindVersion` に応じて区別されます。
