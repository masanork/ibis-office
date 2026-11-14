# Sorane Docs の個人ファイル保存（試作）

Mikaki でログインした人が、自分の Word / Excel / PowerPoint ファイルを Cloudflare 上に保存し、ブラウザ版エディターで開いて書き戻すための試作です。共有リンク、WebDAV、共同編集は対象外です。

この文書は、リポジトリに入ったコードを本番や手元で動かすために運営者が行う作業を書きます。バケットの作成、遠隔 D1 への migration 適用、デプロイ自体は、この変更の中では行っていません。

## 動き

- 画面は `https://docs.sorane.dev/files`（Worker 経路 `/files`）。ホームの「保存したファイル」からも入れます。
- 認証は既存のサンプル管理と同じ Mikaki 機密クライアント（Authorization Code + PKCE）です。callback はこれまでどおり `https://docs.sorane.dev/callback` です。新しい redirect URI は不要です。ログイン後の戻り先だけ、訪問した画面に応じて `/files` か `/admin` になります。
- ファイルの所有者は、この RP における issuer と pairwise `sub` です。他人のファイルは 404 になります。
- API は同オリジンの `/api/files` です。Cookie セッションと CSRF、`Origin` 検査を使います。
- 開くときは `editor.html?open=/api/files/{id}/{name}&download=1` です。書き戻しは埋め込みの `ibisoffice:download` イベントを `www/sorane-files-save.js` が受け、`If-Match` 付きの PUT で行います。衝突すると 412 または 409 を画面に出します。
- ブラウザ版の「ファイル → 保存」はまだクラウドに対応していません。書き戻しはエディター右上のダウンロードボタンを使います。
- 受け付けるのは docx / xlsx / pptx だけで、大きさは 32 MiB までです。ファイル名は半角英数と `.` `_` `-`、80 文字以内です。ZIP の構造だけを見る検査をします（完全な OOXML 検証やマルウェア検査ではありません）。
- 上書きの前に、いまのオブジェクトを `versions/{id}/{unix-ms}` へコピーします。直近 10 世代を残し、それより古い版は削除します。R2 にネイティブのオブジェクトバージョニングはない前提です。

## 運営者が本番で行うこと

1. Cloudflare で R2 バケット `sorane-docs-files` を、このアカウント（`wrangler.production.jsonc` の `account_id`）に作成する。作成するまで本番デプロイは失敗します。バケット名を変える場合は `apps/sorane-docs/admin/wrangler.production.jsonc` の `FILES` バインディングも合わせて直します。
2. 既存の D1 `sorane-docs-admin`（id `e6cf3f43-1afd-4561-8737-c074749ac5ad`）へ、新しい migration を適用する。

   ```sh
   cd apps/sorane-docs/admin
   npx wrangler d1 migrations apply sorane-docs-admin --config wrangler.production.jsonc --remote
   ```

   適用前に `npx wrangler d1 migrations list sorane-docs-admin --config wrangler.production.jsonc --remote` で、`0005_user_files.sql` が未適用であることを確認します。
3. Mikaki の本番 RP（client id `6677906b-561d-4745-8a94-459db819082a`、issuer `https://auth.mikaki.org`、origin `https://docs.sorane.dev`）を確認する。redirect URI は既存の `https://docs.sorane.dev/callback` のままで足しません。ログアウト後の戻りの登録がある場合も、これまでと同じです。
4. 管理 Worker `sorane-docs-admin` をデプロイし、続けてフロントの `wrangler.jsonc`（docs.sorane.dev / ibis.sorane.dev）もデプロイする。フロント側は `/files` と `/api/files*` を `run_worker_first` で管理 Worker へ渡します。この PR ではデプロイしていません。
5. デプロイ後に、Mikaki でログインし、32 MiB 以下の docx をアップロードし、エディターで開いてダウンロードボタンで書き戻し、別セッションからの衝突、削除まで一通り確認する。ibis.sorane.dev の `/files` は docs.sorane.dev へ 308 で飛び、認証 Cookie は docs.sorane.dev に付きます。

## 手元での確認

本番のデータベース・バケット・秘密情報は使いません。

```sh
cd apps/sorane-docs/admin
npm ci
npm run check
npm test
npx wrangler d1 migrations apply sorane-admin-local --config wrangler.local.jsonc --local
```

対話的に試す場合は、サンプル管理と同じく、ローカル用の使い捨て OIDC と `RP_PRIVATE_JWK` を `--env-file` で渡し、`wrangler dev --local --config wrangler.local.jsonc` を使います。手順の詳細は `apps/sorane-docs/admin/README.md` にあります。ローカルの R2 バインディング名は `FILES`（バケット名 `sorane-files-local`）です。

## コードの場所

- API と画面: `apps/sorane-docs/admin/src/files.ts`（入口は `src/index.ts`）
- migration: `apps/sorane-docs/admin/migrations/0005_user_files.sql`
- フロントの経路: `worker/sorane-gateway.ts`、`wrangler.jsonc`
- エディターからの書き戻し: `www/sorane-files-save.js`（`www/editor.html` から読み込み）
