# masanork forkのブランチ運用

このforkで利用する最新版の統合先は`masanork`。
GitHubの既定ブランチも`masanork`とし、機能・修正は個別ブランチで開発する。
`main`はupstream由来の履歴を追うために残し、`gh-pages`はupstreamのplayground用履歴として扱う。

| 対象 | 用途 |
| --- | --- |
| `origin` | `https://github.com/ibisjp/ibis-office`。upstreamの取得元 |
| remote `masanork` | `https://github.com/masanork/ibis-office.git`。既定push先 |
| branch `masanork` | 完成した機能を統合する最新版。追跡先は`masanork/masanork` |
| `feat/*`、`fix/*`等 | 個別の開発。統合後、作業が完了した枝は削除 |

## 開発と統合

```sh
git fetch masanork
git switch masanork
git pull --ff-only
git switch -c feat/example
```

別の変更が残るcheckoutでは、`git worktree add -b feat/example /path/to/worktree masanork/masanork`
で分離する。進行中の変更を機械的にstash、reset、cleanしない。

機能ブランチで必要な検証を終え、`masanork`へmergeする。
競合解消後の統合結果も確認し、その最終HEADをpushする。
CIがある場合は最終HEADの結果を確認してから完了扱いにする。
PRで統合する場合はbaseを`masanork`にする。

完了した枝は、`git merge-base --is-ancestor <feature> masanork`等で取り込みを確認し、
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
cargo test -p ibis-pptx --lib jtd::
python3 -m unittest discover -s scripts -p test_audit_jtd.py -v
scripts/build-web.sh --release
python3 scripts/check-web-assets.py
```

`npm run test:fonts`は、このrepoのsubsetterをビルドして合成フォントで検証する。
追加フォントの取得・R2への配置は[Web fonts](web-fonts.md)を参照。
配信先は<https://docs.sorane.dev>（既存の<https://ibis.sorane.dev>も維持）。配信時はこのブランチの`wrangler.jsonc`を使い、
dry-runの後にdeployし、公開されたmanifestとWASMのhash、ブラウザー動作を確認する。
単なるブランチ整理では、検証中の別作業による配信を上書きしない。

## デスクトップ CI

`Desktop build CI` は `masanork` への push、同ブランチ宛て PR、手動実行で
macOS arm64、Windows x64、Linux x64 AppImage をビルドする。PR はマージ用の
仮コミットではなく PR head の SHA を checkout し、submodule の状態と Rust の
版を成果物の `build-info.json` に記録する。各ジョブはパッケージを検証して
`SHA256SUMS` とともに Actions artifact として14日間保存する。

これらは **CI 検証用**であり、公開リリースではない。macOS は ad-hoc 署名で
公証されておらず、Windows も配布署名はない。現行の名前・ロゴと upstream の
自動更新先も配布向けには整理が必要。タグ付けや GitHub Release への自動添付、
署名鍵、デプロイ権限はこの workflow に含めない。CI の起動テストは異常な引数の
終了コードやパッケージ構造を確認するもので、GUI 操作の検証ではない。
