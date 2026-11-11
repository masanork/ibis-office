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
配信先は<https://ibis.sorane.dev>。配信時はこのブランチの`wrangler.jsonc`を使い、
dry-runの後にdeployし、公開されたmanifestとWASMのhash、ブラウザー動作を確認する。
単なるブランチ整理では、検証中の別作業による配信を上書きしない。
