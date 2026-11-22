# PDF/A 対応

デスクトップ版の既存のベクターPDF出力を使い、PDF/A-2bプロファイルを
指定して出力できます。

```sh
ibisOffice --export pdfa-2b --out-dir ./archive document.docx
```

入力は通常のベクターPDF出力と同じDOCX・XLSX・PPTXです。PDF/Aモードでは
出力ライブラリにPDF/A-2bの条件を適用し、対応するXMP識別情報を付加します。
ライブラリが条件違反を検出した場合は出力に失敗し、通常のPDFには切り替えません。

LinuxのデスクトップCIでは、AppImage内のibisOfficeを使い、合成文書3形式の
PDF/A識別情報とテキスト抽出を確認します。固定したveraPDF 1.30.4で
PDF/A-2b適合、通常のPDFと別の要求プロファイルでの非適合、検証器の起動失敗も
確認します。ローカルで再現する場合は、veraPDFのCLIを指定してください。

```sh
python3 scripts/test-pdfa-export.py target/debug/ibisOffice --verapdf /path/to/verapdf
```

ISO 19005対応は、プロファイルごとの出力と既存PDFの適合性の検証を分けて
進めます。PDF/A-2bの出力に続いて、他のプロファイルと非適合の診断、
既存PDF/Aの読み取り・保存時の適合性の維持を確認します。タグ付きの
プロファイルでは文書の論理構造も扱います。

AcroFormの入力と保存は別の段階です。対応時にはフィールド値に加えて、
別のビューアでも同じ値が見えるようウィジェットの外観を生成し、
保存後に対象プロファイルで再検証します。

## 既存PDFのプロファイルを確認する

```sh
ibisOffice --export pdfa-info document.pdf
```

文書カタログのXMPから `part`、`conformance`、`revision` を読み取り、
判別できたPDF/A-1〜4のプロファイルを `declared_profile` に出力します。
宣言がない場合や組み合わせが未知の場合は `declared_profile` が `null` になります。
XMPの解析エラーや同じ項目の値が矛盾している場合は終了コードでエラーを返します。
UTF-8のXMPに対応し、メタデータストリームの読み取りは1 MiBまでです。

このコマンドの `validation` は `not_run` です。保存や提出の前に適合性を確認する
場合は、提出先が求めるプロファイルを指定して独立検証します。たとえば、
PDF/A-2bが必要な場合は次のように実行します。

```sh
verapdf --flavour 2b document.pdf
```

## 提出先が求めるプロファイルで検証する

```sh
ibisOffice --export pdfa-validate --pdfa-profile 2b document.pdf
```

別途インストールしたveraPDF CLIを使います。PATHにない場合は
`--verapdf /path/to/verapdf` で実行ファイルを指定できます。
プロファイルは必須です。PDF/A-1の `1a`・`1b`、PDF/A-2と3の
`a`・`b`・`u`、PDF/A-4の `4`・`4e`・`4f` を指定できます。

結果はJSONで、適合の場合は `compliant`、違反がある場合は
`non_compliant` を返します。違反ルールごとに規格番号、条項、説明、
失敗した検査の件数、最初の該当箇所を出力します。検証は90秒で打ち切ります。

| 終了コード | 意味 |
| --- | --- |
| 0 | 適合 |
| 3 | 非適合 |
| 1 | PDFを読み取れない、検証器を実行できない、または結果を取得できない |
| 2 | CLIの引数が不正 |
