# 配布物のサプライチェーン検証

対象は ibisOffice の配布経路。SLSA の目標は仕様 1.2 の Build L3 とする。
基準は [Build 要件](https://slsa.dev/spec/v1.2/build-requirements)、
[Build Provenance](https://slsa.dev/spec/v1.2/build-provenance)、
[配布物の検証](https://slsa.dev/spec/v1.2/verifying-artifacts)。

## BOM の対象

| 配布経路 | BOM の根拠 | 追加で必要な確認 |
| --- | --- | --- |
| Linux x64 AppImage | Cargo 通常依存、展開したファイルとリンク、ELF の直接依存名 | ランタイムで解決した OS ライブラリ、動的ロード |
| Linux x64・arm64 Flatpak | Cargo 通常依存、ビルドエクスポート中のアプリファイルと ELF 直接依存名 | 配布バンドルの展開比較、利用する Freedesktop ランタイムの版と BOM |
| macOS・Windows | Cargo 通常依存と配布 ZIP のファイル、SHA-256、リンク | Mach-O と PE のインポート、OS や SDK の提供物 |
| Android・iOS | 対象アーキテクチャの Cargo 通常依存と APK／アプリ ZIP の内容 | NDK、Apple SDK、Java／Objective-C ブリッジ、OS 暗号 API |
| WASM | エディターと HEIC デコーダーの Cargo 通常依存、CI で生成した WASM／グルーの ZIP | 本番 Web 配布物、Worker の依存、フォント・素材、ブラウザー API、本番で配信したバイトとの照合 |

CBOM は各ターゲットの Cargo グラフで確認できたコンポーネントについて、
OOXML／PDF 暗号化および OOXML／OFD の署名検証をソースに基づいて記録する。
追加調査はアップデーター、TLS、証明書の信頼と失効確認、プラットフォーム委譲、
Worker 側の暗号処理を対象にする。利用宣言と実行時に観測した処理を個別に記録する。

## SLSA Build フィットギャップ

| 要件 | 現状 | 次の作業と受け入れ条件 |
| --- | --- | --- |
| Build L1 の provenance | Desktop CI にソース SHA、ツールチェーン、成果物ハッシュがある。WASM と Flatpak は経路が異なる | 配布ファイルごとに subject digest、source URI／commit、build type、builder ID、起動パラメータを含む in-toto statement を出す。タグ付きリリースの配布ファイルすべてに対応する証跡を照合する |
| Build L2 の真正性 | GitHub ホストランナーでビルドし、一部の成果物にプラットフォーム署名がある。現状の BOM とビルド情報はジョブが生成する | 採用するビルダーの信頼境界を確認し、ホスト側で生成・認証した provenance を配布する。別環境で署名、issuer、repository、workflow、source ref、digest を検証し、別リポジトリ由来や改変ファイルを拒否する |
| Build L3 の偽造耐性 | ビルドとリリースを別ジョブに分けている。ビルド側の出力をもとにリリースが構成される | 署名権限をビルド環境から分離し、制御面で検証できる入力から provenance を生成する。任意のビルド出力で署名対象と provenance を差し替えられないことを確認する |
| Build L3 の隔離 | ホストランナーを用い、一部でコンテナとキャッシュを使う | ビルダーの隔離保証を確認し、PR キャッシュ、ワークフロー、再利用ジョブ、手動入力が後続リリースに影響できる経路を検証する。外部パラメータと昇格条件を列挙する |
| 入力の特定と配布側の照合 | Cargo.lock と npm lockfile があり、Desktop CI は固定 Rust を選ぶ。経路によって可変のコンテナタグ、Actions 参照、OS パッケージやツール配布がある | Actions／イメージを検証済みの SHA／digest で参照する。Rust、サブモジュール、SDK／NDK、システムパッケージ、外部ダウンロードの確定した識別子を記録する。Web は配備後に公開 digest と CI の証跡を照合する |

まず Linux AppImage でリリースに付随する証跡を発行し、
ダウンロードした配布物を別環境で検証するところまで通す。
その後、同じ受け入れ条件を macOS、Windows、モバイル、Flatpak、Web に適用する。
BOM の CI はスキーマと参照整合性、配布物とのハッシュ照合、ibisOffice の XLSX 変換を
対象とし、実物に加えて期待値を固定した合成フィクスチャも維持する。


## Linux AppImage の署名境界と隔離

| Build L3 の確認点 | 実装と検証方法 | 評価の残り |
| --- | --- | --- |
| 署名権限の分離 | 固定コミットの再利用ワークフロー内で、ビルドは `contents: read`、署名は別ランナーの `id-token: write` と `attestations: write`。ビルド時に OIDC 要求用の環境変数がないことを確認する | 署名用ワークフローの更新権限とレビュー運用の監査 |
| 来歴への入力 | ビルダーの runner、checkout ref の決め方、成果物名を固定する。署名ランナーは checkout や成果物の実行を行わず、同じ実行から取得した AppImage を `actions/attest` に渡す。呼び出し側の手動入力は他プラットフォームの実行有無だけを変える | GitHub の制御面と外部ビルド入力の記録範囲を評価する |
| ビルド間の隔離 | GitHub ホストランナーと固定 OCI digest を使い、共有ビルドキャッシュと他の実行からの成果物取り込みを使わない。`target` と成果物ディレクトリが存在しない状態から始める。チェックアウト認証情報を永続化しない | ホストランナーの分離保証、ネットワーク、ランナーイメージと APT／外部ツールの入力管理を継続評価する |
| 配布後の検証 | 別ランナーで AppImage と bundle を取り直し、ファイルの digest、ソース ref と SHA、再利用ワークフローのパスとコミットを照合する | 上流や別の配布経路へ展開する際の信頼するビルダーと ref の運用を決める |

ビルダーの更新時は、再利用ワークフローの差分を確認したうえで、呼び出し側と
配布手順で用いる pin を一緒に更新する。これらはこの経路の確認範囲であり、
Build L3 の適合評価は上表の残項目を含めて行う。

[GitHub の再利用ワークフローとビルド来歴](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/increase-security-rating)
