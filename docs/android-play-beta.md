# Android 版 soraneDocs の Google Play Console ベータ配信手順

Android 版 soraneDocs（Application ID: `dev.sorane.docs`）を Google Play Console の内部テスト（`internal`）・クローズドテスト（`alpha`）・オープンテスト（`beta`）へ配信するための手順と前提設定です。

## 1. 仕様とバージョン採番

| 項目 | 値 |
| --- | --- |
| Application ID | `dev.sorane.docs`（公開後は変更不可） |
| Activity | `dev.sorane.docs.IbisActivity` |
| `minSdkVersion` / `targetSdkVersion` | 26 / 36 |
| ABI | `arm64-v8a`（16 KB ページサイズアラインメント検証付き） |
| `versionName` | `version.txt` の `Android` 行（例: `0.8.9`） |
| `versionCode` | `git rev-list --first-parent --count HEAD`（`ANDROID_VERSION_CODE` で上書き可） |

## 2. アップロード鍵（Upload Keystore）の作成（初回のみ）

Google Play アプリ署名（Play App Signing）を利用するため、永続的なアップロード鍵をローカルで 1 回作成し、安全な場所（パスワードマネージャー等）に保管します。**Git には絶対にコミットしません。**

```sh
keytool -genkeypair -v \
  -keystore sorane-docs-upload.jks \
  -storetype PKCS12 \
  -alias sorane-upload \
  -keyalg RSA -keysize 2048 \
  -validity 10000 \
  -dname "CN=soraneDocs,O=sorane.dev,C=JP"
```

GitHub Actions へ登録する Base64 文字列を生成します：

```sh
base64 -i sorane-docs-upload.jks | tr -d '\n' | pbcopy
```

## 3. Google Play Console の API サービスアカウント設定

1. Google Cloud Console でプロジェクトを選択し、**Google Play Android Developer API** を有効化します。
2. サービスアカウントを作成し、JSON キーを発行します。
3. Google Play Console の **ユーザーと権限** → **新しいユーザーを招待** から上記サービスアカウントのメールアドレスを追加し、対象アプリ（`dev.sorane.docs`）に **リリースの管理（テストトラックへのリリース作成・編集）** 権限を付与します。

## 4. GitHub Secrets の登録

リポジトリ（`sorane-dev/docs`）の **Settings → Secrets and variables → Actions** に以下 5 件を登録します。

| Secret 名 | 内容 |
| --- | --- |
| `ANDROID_UPLOAD_KEYSTORE_BASE64` | `sorane-docs-upload.jks` の Base64（改行なし） |
| `ANDROID_UPLOAD_KEY_ALIAS` | 鍵のエイリアス（例: `sorane-upload`） |
| `ANDROID_UPLOAD_KEYSTORE_PASS` | キーストアのパスワード |
| `ANDROID_UPLOAD_KEY_PASS` | 鍵のパスワード（キーストアと同じ場合は省略可） |
| `GOOGLE_PLAY_SERVICE_ACCOUNT_JSON` | サービスアカウントの JSON キー全文 |

## 5. 初回リリースと自動デプロイの流れ

> **重要（Google Play API の仕様）**: 新規アプリの**最初の 1 回目の AAB** は Play Console の Web UI から手動アップロードする必要があります（API は未作成アプリへ初回アップロードできません）。

1. GitHub Actions の **Android Play Console deploy** ワークフローを `dry_run: true` で手動実行します。
2. 成果物 `android-play-aab-<versionCode>` から署名済み `.aab` をダウンロードし、Play Console の **内部テスト** トラックへ初回のみ手動アップロードして Play App Signing を有効化します。
3. 2 回目以降は **Android Play Console deploy** ワークフローを `dry_run: false` で実行します：
   - `track`: `internal`（既定・内部テスト）/ `alpha`（クローズドテスト）/ `beta`（オープンテスト）
   - `release_status`: アプリ審査完了前は `draft`、公開後は `completed`
4. ローカルで直接 `.aab` をビルド・検証する場合：

```sh
ANDROID_KEYSTORE_PATH=/path/to/sorane-docs-upload.jks \
ANDROID_KEY_ALIAS=sorane-upload \
ANDROID_KEYSTORE_PASS='...' \
scripts/build-android.sh --release --aab --require-release-keystore
```
