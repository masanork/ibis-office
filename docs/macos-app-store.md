# Mac App Store（MAS）配布 CI パイプライン

`soraneDocs`（Bundle ID: `dev.sorane.docs`）を Mac App Store 向けに App Sandbox 付きでビルド・署名・`.pkg` 化し、App Store Connect へアップロードするための手順です。

---

## 1. 構成ファイル
- [mas.entitlements](../crates/ibis-office-app/macos/mas.entitlements): App Sandbox（`com.apple.security.app-sandbox`）およびユーザー選択ファイル・ネットワーク・印刷権限。
- [bundle-macos.sh](../scripts/bundle-macos.sh): `--mas` 指定時は自己アップデータ（`ibisOfficeUpdater`）を除外し、`LSApplicationCategoryType`（`public.app-category.productivity`）付与、プロビジョニングプロファイル同梱、Sandbox 署名、および `productbuild` による `.pkg` 生成を行います。
- [mas-release.yml](../.github/workflows/mas-release.yml): 手動実行（`workflow_dispatch`）の Mac App Store 用 CI ワークフロー。

---

## 2. GitHub Actions Secrets に登録する資材

Apple Developer Connection / App Store Connect で以下を発行し、リポジトリの **Settings → Secrets and variables → Actions** へ登録してください。

| Secret 名 | 内容 | 取得方法 |
| :--- | :--- | :--- |
| `MAS_APP_CERT_P12_BASE64` | **Apple Distribution**（または 3rd Party Mac Developer Application）証明書の `.p12` を base64 化した文字列 | `base64 -i mas-app.p12 \| pbcopy` |
| `MAS_APP_CERT_PASSWORD` | 上記 `.p12` のエクスポートパスワード | |
| `MAS_INSTALLER_CERT_P12_BASE64` | **Mac Installer Distribution**（または 3rd Party Mac Developer Installer）証明書の `.p12` を base64 化した文字列 | `base64 -i mas-installer.p12 \| pbcopy` |
| `MAS_INSTALLER_CERT_PASSWORD` | 上記 `.p12` のエクスポートパスワード | |
| `MAS_PROVISIONING_PROFILE_BASE64` | `dev.sorane.docs` 用の Mac App Store Provisioning Profile（`.provisionprofile`）を base64 化した文字列 | `base64 -i Docs.provisionprofile \| pbcopy` |
| `ASC_API_KEY_ID` | App Store Connect API Key ID | App Store Connect → ユーザーとアクセス → 統合 |
| `ASC_API_ISSUER_ID` | App Store Connect Issuer ID（UUID） | 同上 |
| `ASC_API_KEY_P8_BASE64` | ダウンロードした `AuthKey_*.p8` を base64 化した文字列 | `base64 -i AuthKey_XXXX.p8 \| pbcopy` |

---

## 3. 実行方法
1. **ドライラン検証（署名なしでも実行可能）**:
   - GitHub Actions → **Mac App Store CI** → `Run workflow`（`upload: false`）
   - `--mas` バンドル・Sandbox Entitlements・`ibisOfficeUpdater` 非同梱・`Info.plist` を検証します。
2. **App Store Connect へ提出**:
   - Secrets 登録後、`upload: true`（必要なら `build_number` 指定）で実行すると、`Docs-mas.pkg` を生成して `xcrun altool --upload-app` で提出します。
