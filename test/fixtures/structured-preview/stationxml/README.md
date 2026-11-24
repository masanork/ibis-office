# FDSN StationXML 1.2 の表示サンプル

米国地質調査所（USGS）の[観測所情報システム](https://www.usgs.gov/publications/station-information-system-sis-a-centralized-seismic-station-repository-populating)で使われる
[FDSN StationXML 1.2](https://www.fdsn.org/xml/station/) を、
ibisOfficeの共有XML表示定義で読む試用例です。
観測網、観測所名、座標は試験用の合成データです。

1. ibisOfficeデスクトップ版で [stations.xml](stations.xml) を開きます。
   対応する共有定義が未登録なら、通常のXMLプレビューに要素名と値が表示されます。
2. **設定 → 一般 → 表示定義フォルダーを開く** を選び、
   [stationxml-1.2.view.json](stationxml-1.2.view.json) をそのフォルダーにコピーします。
3. 最初のXMLを閉じて開き直します。「地震観測所メタデータ（合成サンプル）」の下に
   北丘観測所と南谷観測所が別々の明細として表示されます。
   日本語の項目名で緯度・経度を確認でき、標高はそれぞれ 120 m と 240 m です。

定義は名前空間URI、ルート要素名 `FDSNStationXML`、
ルート属性 `schemaVersion="1.2"` で選択されます。
サンプルの構造は公開XSDで再検証できます。このフォルダーで次を実行します。

```sh
schema_file=$(mktemp)
curl -fsSL https://www.fdsn.org/xml/station/fdsn-station-1.2.xsd -o "$schema_file"
xmllint --nonet --noout --schema "$schema_file" stations.xml
rm "$schema_file"
```
