# ブラウザー版の PWA とローカルファイル

soraneDocs（docs.sorane.dev / ibis.sorane.dev）はインストール可能なウェブアプリです。回線が切れても、すでに開いた手元のファイルを編集し続けられるようにしています。データの正本はローカルのファイルそのもので、Sorane のクラウドへは同期しません。

## サービスワーカー

`scripts/build-web.sh` の最後で `scripts/build-pwa-sw.mjs` が `www/sw.js` を生成します。プリキャッシュの一覧は、そのビルドの wasm マニフェストとフォントチャンク（ファイル名にハッシュが付いているもの）と、HTML / JS のシェルから作ります。キャッシュ名は `sorane-docs-<revision>` で、ビルドごとに別の箱になり、有効化のときに古い箱を捨てます。同じ版の JS と wasm だけが組になるので、新しいシェルに古い wasm が混ざることはありません。

- 画面遷移とハッシュのないシェル（`index.html`、`main.js` など）は **network-first** です。オンラインなら新しい版を取り、オフラインならキャッシュ、それも無ければ `offline.html` を出します。
- ハッシュ付きの wasm チャンクとフォントは **cache-first** です。
- `/auth/`、`/api/`、他オリジン、POST はキャッシュしません。`sw.js` 自身もキャッシュしません。
- 新しいサービスワーカーは待たせたままにし、画面の「新しいバージョンがあります。再読み込み」を押したときだけ `skipWaiting` します。

プリキャッシュの大きさはビルド時に `www/pwa-precache.json` へ書き出します。圧縮した wasm シングルトンと既定のフォントチャンク、HTML / JS のシェルを合わせて、この執筆時点のリリースビルドではおよそ 35 MiB です。非圧縮の wasm チャンクは入れません。Cloudflare Worker は `/sw.js` を `Cache-Control: no-cache` と `Service-Worker-Allowed: /` で返します。

## ローカルファイルへの上書き保存

Chromium 系（Chrome / Edge）では File System Access API を使います。`ファイル > 開く`、ファイルのドロップ、インストールしたアプリからの起動で得た `FileSystemFileHandle` を覚え、保存（⌘S / Ctrl+S）はそのファイルへ上書きします。権限がなければその場で尋ねます。名前を付けて保存は `showSaveFilePicker` です。

Safari と Firefox にはこの API が無いので、これまでどおりダウンロード保存にします。書き出し（HTML や PDF など）も、これまでどおりダウンロードです。

## OS からファイルを開く

マニフェストの `file_handlers` に、アプリが開ける主な種類（Word / Excel / PowerPoint、Markdown、OpenDocument の表とスライド、Visio、EPUB、PDF など）を載せています。インストールした PWA を .docx や .md の開き先にできます。開いたハンドルは、そのまま上書き保存に使います。この関連付けは Chromium 系のデスクトップ向けです。

`.odt` は、いまのところ開ける対象に含まれていません。
