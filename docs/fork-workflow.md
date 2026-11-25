# Sorane forkのブランチ運用

このforkで利用する最新版の統合先は`sorane`。
GitHubの既定ブランチも`sorane`とし、機能・修正は個別ブランチで開発する。
`main`はupstream由来の履歴を追うために残し、`gh-pages`はupstreamのplayground用履歴として扱う。

| 対象 | 用途 |
| --- | --- |
| `origin` | `https://github.com/ibisjp/ibis-office`。upstreamの取得元 |
| remote `masanork` | `https://github.com/sorane-dev/docs.git`。既定push先 |
| branch `sorane` | 完成した機能を統合する最新版。追跡先は`masanork/sorane` |
| `feat/*`、`fix/*`等 | 個別の開発。統合後、作業が完了した枝は削除 |

現行の機能差分と upstream 統合後の確認結果は [Docs と upstream ibisOffice の差分](fork-feature-differences.md) を参照。

## 開発と統合

```sh
git fetch masanork
git switch sorane
git pull --ff-only
git switch -c feat/example
```

別の変更が残るcheckoutでは、`git worktree add -b feat/example /path/to/worktree masanork/sorane`
で分離する。進行中の変更を機械的にstash、reset、cleanしない。

機能ブランチで必要な検証を終え、`sorane`へmergeする。
競合解消後の統合結果も確認し、その最終HEADをpushする。
CIがある場合は最終HEADの結果を確認してから完了扱いにする。
PRで統合する場合はbaseを`sorane`にする。

完了した枝は、`git merge-base --is-ancestor <feature> sorane`等で取り込みを確認し、
remoteとlocalの両方から削除する。cherry-pick済みの枝はpatchの対応も確認する。
その枝のworktreeに未コミット変更や進行中の作業があれば、先に保全・引継ぎする。
PRのmerge時にはGitHubのhead branch自動削除も有効にする。

## Gitの名義

以下はこのrepoのローカル設定として適用する。worktree間で共有される。

```sh
git config --local user.name masanork
git config --local user.email 2642715+masanork@users.noreply.github.com
git config --local user.useConfigOnly true
git config --local remote.pushDefault masanork
```

グローバル設定の職場メールに戻さない。cherry-pickは元のAuthorを保持するため、
過去の自作コミットを取り込む場合はAuthor・Committerの両方を確認する。
upstreamの作者情報は書き換えない。

## 検証と配信

変更に応じてWeb・JTDの検証を選ぶ。両機能の統合時には少なくとも次を確認する。

```sh
npm run test:web
cargo test -p ibis-formats --lib jtd::
python3 -m unittest discover -s scripts -p test_audit_jtd.py -v
scripts/build-web.sh --release
python3 scripts/check-web-assets.py
```

Web フォントの静的チャンク配信（SIL OFL 1.1: `ACGJMincho`・`BIZUDGothic`・`BIZUDMincho`）は `npm run build:font-subsets`（または `IBIS_WEB_FONT_SUBSETS=1 scripts/build-web.sh --release`）で `tools/ibis-font-subsetter` から `www/fonts/` と `www/loader-manifest.json` を生成する（[Web 埋め込み](web-embedding-ja.md)）。
配信先は<https://docs.sorane.dev>（既存の<https://ibis.sorane.dev>も維持）。配信時はこのブランチの`wrangler.jsonc`を使い、
dry-runの後にdeployし、公開されたmanifestとWASMのhash、ブラウザー動作を確認する。
単なるブランチ整理では、検証中の別作業による配信を上書きしない。

## デスクトップ CI

`Desktop build CI` は `sorane` への push、同ブランチ宛て PR、手動実行で
macOS arm64、Windows x64、Linux x64 AppImage をビルドする。PR はマージ用の
仮コミットではなく PR head の SHA を checkout し、submodule の状態と Rust の
版を成果物の `build-info.json` に記録する。各ジョブはパッケージを検証して
`SHA256SUMS` とともに Actions artifact として14日間保存する。

これらは **CI 検証用**であり、公開リリースではない。macOS は ad-hoc 署名で
公証されておらず、Windows も配布署名はない。通常のブランチ CI は GitHub Release を作成せず、署名鍵やデプロイ権限も含めない。
CI の起動テストは異常な引数の
終了コードやパッケージ構造を確認するもので、GUI 操作の検証ではない。

### 配布物の SBOM と CBOM

Desktop build CI は Linux AppImage、macOS、Windows、Android、iOS の
CycloneDX 1.6 SBOM と CBOM を CI 成果物に含める。
Flatpak の手動ビルドでは x64／arm64 の成果物、Web CI では `wasm-ci-bom` に
生成した WASM とグルーの ZIP およびその BOM を収録する。
BOM にはソース SHA、Cargo ターゲット、対象パッケージの SHA-256 と棚卸し範囲がある。

Linux AppImage、macOS、Windows の Desktop CI はパッケージから取り出した ibisOffice で
両 BOM を XLSX に変換し、期待するシートを確認する。
配布経路ごとの対象と SLSA Build L3 に向けた差分は
[配布物のサプライチェーン検証](supply-chain-fit-gap.md)を参照。

### Linux AppImage の署名付きビルド来歴

`sorane` の push、対象タグ、手動実行では、AppImage をビルドしたジョブから
別の GitHub ホストランナーに成果物を渡し、GitHub Artifact Attestations で署名する。
Linux のビルドと署名は、コミットを固定した再利用ワークフローが実行する。
署名権限はその中の署名ジョブに付与し、PR 実行は読み取り専用の呼び出しを使う。
署名ジョブはコードを checkout せず、ビルドが返した不変の artifact ID で同じ実行の AppImage を取得してハッシュする。
署名した証跡は `desktop-ci-linux-x64-provenance` に収録し、
マイルストーン Release では AppImage と同じ場所に置く。
手動で Linux だけを検証する場合は Desktop build CI の `appimage_only` を選ぶ。

検証時は、信頼するコミットと Git ref を指定する。タグ付き Release の場合は
`refs/tags/sorane-v…`、統合ブランチのビルドでは `refs/heads/sorane` を使う。

```bash
gh attestation verify desktop-linux-x64-ci.AppImage \
  --bundle desktop-linux-x64-ci.AppImage.intoto.jsonl \
  --repo sorane-dev/docs \
  --signer-workflow sorane-dev/docs/.github/workflows/linux-appimage-builder.yml \
  --signer-digest 0ea0a290aaef5c68c8a2160d7ada061a3c6d5d93 \
  --source-digest "$EXPECTED_SOURCE_SHA" --source-ref "$EXPECTED_REF" \
  --deny-self-hosted-runners
```

CI はさらに別のランナーへ AppImage と bundle をダウンロードし、署名、digest、
出所を検証する。改変した AppImage、異なる ref、異なるビルダーコミットが拒否されることも確認する。
この段階で検証するのは GitHub ホストランナー上の署名付き来歴と AppImage の結び付きである。
[SLSA Build L3 の評価項目](supply-chain-fit-gap.md)として、信頼境界と実行間の隔離の確認を続ける。

## マイルストーン Release

`sorane` の履歴にある検証済みコミットに `sorane-v1.2.3` のようなタグを
push すると、Desktop build CI が同じコミットの Linux x64 AppImage、Flatpak x86_64／arm64、
macOS arm64、Windows x64、Android arm64、iOS arm64 をビルド・検証し、
全ジョブ成功後に GitHub Release を作成する。
`-rc.1` のようなハイフン付き接尾辞は prerelease として公開される。
上流タグと区別するため `sorane-v` を使う。

```sh
git fetch masanork sorane
git tag -a sorane-v1.2.3 masanork/sorane -m 'Docs milestone 1.2.3'
git push masanork sorane-v1.2.3
```

Release には各パッケージ、対応する SBOM／CBOM、ビルド情報、SHA256SUMS を添付する。
Flatpak はパッケージと BOM を同じタグのビルドで生成する。タグが指すコミットが
公開時にも `sorane` の履歴にあることを確認し、既存 Release は上書きしない。
タグを付け替えて再利用せず、修正には新しいタグを使う。Release は自動公開される。
macOS は公証されず Windows は未署名であり、自動更新フィードではない。
公開前には対象コミットの CI と配布内容を確認する。

## Desktop branding and updates

Desktop builds use the Docs name and the document D mark in `crates/ibis-office-app/branding/`. The macOS bundle and Linux desktop application have their own identifiers so they can coexist with ibisOffice. About retains the required Powered by ibisOffice attribution.

Automatic and manual in-app update checks are disabled in this fork: the inherited updater points at upstream ibisOffice releases, not Docs builds. Do not re-enable it until a separate fork release feed and verification policy are in place. Desktop CI artifacts are validation builds, not an update feed. Windows automatic Open With registration is also disabled until the fork has a distinct executable name; this avoids overwriting an upstream installation’s registration.

Signed update metadata preparation and the remaining production activation gates are documented in [Docs desktop updates](sorane-desktop-updates.md).

## Docs のビルド番号

Docs のビルド名は `docs-v0.8.9-build862` の形式を使う。
番号は対象コミットの `git rev-list --first-parent --count HEAD` で、
同じコミットの再実行や各プラットフォームのビルドで共通になる。
序列として使う範囲は `sorane` の first-parent 履歴。
他のブランチでは同じ番号になることがあるため、成果物の照合には
完全なソースコミットと SHA-256 を使う。

Help のバージョン情報は `0.8.9 (build 862)` のように表示する。
CI の BOM とビルド記録にも番号と名前を保存する。Git 履歴を取得できない
手元のビルドではビルド番号を省略し、CI は完全な履歴を取得してからビルドする。
