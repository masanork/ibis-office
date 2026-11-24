# Linux AppImage の SBOM・CBOM レビュー

同じ配布形式の CI 成果物を2版選び、依存関係と暗号資産の差分を ibisOffice で確認する。
実例では [旧版の CI](https://github.com/masanork/ibis-office/actions/runs/45313339537) と
[新版の CI](https://github.com/masanork/ibis-office/actions/runs/45341383033) を使った。
CI 成果物には保存期限があるため、期限後は取得可能な別の2版を選ぶ。

```sh
gh run download 45313339537 -R masanork/ibis-office -n desktop-ci-linux-x64 -D before
gh run download 45341383033 -R masanork/ibis-office -n desktop-ci-linux-x64 -D after
(cd before && shasum -a 256 -c SHA256SUMS)
(cd after && shasum -a 256 -c SHA256SUMS)

# OSV-Scanner v2.7.1。検出ありの終了コード 1 も許容する。
for release in before after; do
  rc=0
  osv-scanner scan source --format json -L "$release/linux-x64-appimage.sbom.cdx.json" \
    >"$release/osv.json" || rc=$?
  [ "$rc" -le 1 ] || exit "$rc"
  jq -e '.results | type == "array"' "$release/osv.json" >/dev/null || exit 1
done

ibisOffice --export sbom-review \
  --old-sbom before/linux-x64-appimage.sbom.cdx.json --old-scan before/osv.json \
  --new-sbom after/linux-x64-appimage.sbom.cdx.json --new-scan after/osv.json \
  --out-file dependency-review.xlsx
ibisOffice --export cbom-compare \
  --old-sbom before/linux-x64-appimage.cbom.cdx.json \
  --new-sbom after/linux-x64-appimage.cbom.cdx.json --out-file crypto-changes.xlsx
```

署名付きの版を入手した場合は、[フォークの運用手順](fork-workflow.md)に沿って
AppImage の署名付きビルド来歴も検証する。スキャン時点のアドバイザリと判断を再現
できるよう、両版の BOM、OSV JSON、スキャン日時とツールのバージョンを保存する。

`dependency-review.xlsx` の `Findings` に担当者、期限、判断、判断理由、証拠を
記入して保存する。次の OSV JSON を「ファイル → 脆弱性レビューを更新…」で読み込むと、
エコシステム、パッケージ、OSV ID が一致した行の記入内容を引き継いだ新しいブックが開く。
別名で保存して、記入内容と再確認フラグを確認する。

2026-11-24 の比較では、両版でスキャナーが認識した PURL 付きパッケージは772件ずつで、
31件のパッケージ・バージョン・アドバイザリの組み合わせが15パッケージで一致した。
新版では AppImage の同梱ファイル201件と直接 ELF 依存名9件が目録に加わり、
暗号資産の記載は3件から17件に増えた。BOM に初めて現れた項目を読む際は、
生成時の収集範囲と実際の製品変更をそれぞれ確認する。

このスキャンで確認できる範囲は、OSV が照合できたパッケージとバージョン。
OS パッケージや共有ライブラリのアドバイザリには別途識別情報が必要で、
検出を評価する際は、該当コードへの到達性と配布形態を確認する。

## デスクトップ更新の暗号境界

デスクトップ CBOM は、対象の Cargo グラフに更新機能が含まれる場合に、更新マニフェストの
Ed25519 検証とダウンロードファイルの SHA-256 照合を記録する。
HTTPS 取得は実行環境の `curl` に委譲するため、TLS バックエンド、交渉された
プロトコル、証明書ストアは実行環境で確認する。自動更新の有効化条件は
[デスクトップ更新の手順](sorane-desktop-updates.md)を参照。

## 対象プラットフォームと CI の検証

| 対象 | 照合する CI 成果物 | 同梱物の検証 |
| --- | --- | --- |
| Linux x64 | AppImage | 展開したファイルと直接 ELF 依存名を収集し、別ランナーで AppImage のハッシュと署名付き来歴を検証 |
| macOS arm64 | `Docs.app` の ZIP | 実行ファイルと更新ヘルパーを含む ZIP の全ファイルとシンボリックリンクを照合 |
| Windows x64 | ibisOffice の ZIP | 実行ファイルと更新ヘルパーを含む ZIP の全ファイルを照合 |
| iOS arm64 | デバイス向け `IbisOffice.app` の ZIP | 実行ファイルと `Info.plist` を含む ZIP の全ファイルを照合 |
| Android arm64 | APK | マニフェスト、DEX、arm64 共有ライブラリを含む APK の全ファイルを照合 |
| WASM | リリースビルドの `www/pkg` の ZIP | エディター、HEIC モジュール、生成された JS を収集し、マニフェストに沿ってエディターの全チャンクと再結合結果を照合 |

いずれも CI 成果物を別ランナーにダウンロードし、CycloneDX 1.6 のスキーマ、
参照整合性、ソースコミット、パッケージの SHA-256 を確認する。
ZIP と APK では格納された各ファイルの SHA-256 とシンボリックリンクのリンク先も
両 BOM と突き合わせる。Windows・macOS は同梱 ibisOffice で両 BOM を XLSX に変換して確認する。
WASM の成果物はブラウザー向けモジュールと JS の CI リリースビルドで、Worker や配信サイト全体の
インベントリは別途必要になる。ZIP／APK のネイティブライブラリの実行時解決先も別途確認する。

macOS の CI 成果物はアドホック署名、Windows は未署名。
モバイルの成果物は CI ビルドであり、ストアの配信物と照合する場合はその配信物を入力とする。

## 公開ファイルとの照合

Flatpak は、公開先から取得したバンドルと同じビルドの SBOM／CBOM を同じ
ディレクトリに置いて照合する。Linux 側に Flatpak と OSTree が必要。
検証コマンドは一時 OSTree リポジトリにバンドルを取り込み、アプリを実行せずに
アーキテクチャ、宣言されたランタイム、収録ファイルを確認する。ランタイム自体の
構成とバージョン解決は別途確認する。

```sh
python3 -m pip install 'jsonschema==4.23.0'
python3 scripts/verify-build-boms.py flatpak-x64 downloaded-x64 "$source_sha"
python3 scripts/verify-build-boms.py flatpak-arm64 downloaded-arm64 "$source_sha"
```

Web は、公開したビルドの CI ZIP と両 BOM を入力とし、実際に配信される
`pkg/` のファイルを再取得して全 SHA-256 を照合する。

```sh
python3 scripts/verify-build-boms.py wasm downloaded-web "$source_sha" \
  --public-web-base https://ibis.sorane.dev
```

2026-11-24 に [上流 v0.8.9](https://github.com/ibisjp/ibis-office/releases/tag/v0.8.9)
から公開 Flatpak 2種を取得し、添付の SHA-256 と照合して OSTree へ展開した。
両方とも `org.freedesktop.Platform//24.08` を宣言し、アプリの同梱物は各20件だった。
この Release に BOM は添付されていない。
[フォークの Flatpak CI](https://github.com/masanork/ibis-office/actions/runs/45345837385)
にも同じバージョン名の成果物があるため、照合にはバージョン名と併せて
ハッシュとソースコミットを使用する。

本番 Web のマニフェストに載っている4チャンクと Brotli 版を取得し、いずれからも
SHA-256 `5b449e836986a3fec7ce3b5be40361fb94188947dd14f625e649167144b8ed62`
の WASM を復元できた。上流 v0.8.9 の Web ZIP と既存フォーク CI の WASM は
どちらも異なる SHA-256 だった。公開検証を通すには、実際にデプロイしたビルドの
ZIP と BOM を成果物として保存し、デプロイ後の照合入力に渡す。
