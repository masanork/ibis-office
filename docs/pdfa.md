# PDF/A 対応

デスクトップ版の既存のベクターPDF出力を使い、PDF/A-2bプロファイルを
指定して出力できます。

```sh
ibisOffice --export pdfa-2b --out-dir ./archive document.docx
```

入力は通常のベクターPDF出力と同じDOCX・XLSX・PPTXです。PDF/Aモードでは
出力ライブラリにPDF/A-2bの条件を適用し、対応するXMP識別情報を付加します。
ライブラリが条件違反を検出した場合は出力に失敗し、通常のPDFには切り替えません。

LinuxのデスクトップCIでは、合成文書3形式に対してPDF/A識別情報とテキスト抽出を
確認します。独立検証を行う場合は、veraPDFのCLIを指定してください。

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
