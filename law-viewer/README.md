# Sorane Docs 法令履歴

[law.sorane.dev](https://law.sorane.dev/) で法令を検索し、二つの版を比較できます。表示は新旧対照条文、各版の本文、改め文、要綱、理由、参照条文です。新旧対照条文・五点セットのDOCXと差分JSONはブラウザー内のRust/WASMで生成します。PDFは選択中の表示をブラウザーの印刷機能で保存します。組版済みの五点セットPDFが必要な場合は、セット版DOCXをDocsで開いてPDFに書き出します。

検索と版の一覧、比較用の法令XMLは e-Gov 法令APIを利用します。選択した法令のGit本文・履歴と、対応が取れた新版・旧版のGit本文は [houritsu](https://github.com/Kohei-Wada/houritsu) の索引からリンクします。索引はGitのツリーとコミットメタデータから作り、更新には再ビルドが必要です。共有URLは e-Gov 版IDで比較対象を記録します。

## ビルドと公開

Rustの `wasm32-unknown-unknown` ターゲット、`wasm-bindgen-cli` 0.2.128、Node.js、Python 3、Gitが必要です。リポジトリのルートで実行します。

```sh
git submodule update --init --recursive
npm ci
git clone --bare --filter=blob:none --single-branch https://github.com/Kohei-Wada/houritsu.git /tmp/houritsu.git
git -C /tmp/houritsu.git fetch --filter=blob:none origin +refs/heads/upcoming:refs/heads/upcoming
law-viewer/build.sh /tmp/houritsu.git
npx wrangler deploy --config law-viewer/wrangler.jsonc --dry-run
npx wrangler deploy --config law-viewer/wrangler.jsonc
node law-viewer/browser-test.mjs https://law.sorane.dev/
```

ローカルでは `npx wrangler dev --config law-viewer/wrangler.jsonc --port 8789` を使います。ブラウザーテストはPlaywrightのChromiumを使い、`CHROME_BIN` で実行ファイルを指定できます。
