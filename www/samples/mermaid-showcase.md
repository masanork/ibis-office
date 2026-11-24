# Mermaid 構文デモ集（UML・アーキテクチャ・チャート総集編）

この文書は、Docs の Markdown ビューアー／エディターで描画できる **UML を含む主要な Mermaid 22 図種** を 1 ページにまとめた総集編サンプルです。各コードブロックを編集すると、図の表示も確認できます。

---

## 第 1 部：UML 図（構造・振る舞い）

### 1. クラス図（Class Diagram）

```mermaid
classDiagram
    class Document {
        +String title
        +Kind kind
        +bool dirty
        +open(bytes)
        +exportPdf()
    }
    class WordDocument {
        +Paragraph[] paragraphs
        +Table[] tables
    }
    class Workbook {
        +Sheet[] sheets
        +recalculate()
    }
    class Presentation {
        +Slide[] slides
    }
    Document <|-- WordDocument
    Document <|-- Workbook
    Document <|-- Presentation
```

### 2. シーケンス図（Sequence Diagram）

```mermaid
sequenceDiagram
    actor User as 利用者
    participant App as Docs (WASM)
    participant Server as 配信サーバー

    User->>App: サンプルを選択
    App->>Server: 文書データを取得
    Server-->>App: バイナリ応答
    App->>App: 文書を解析・必要文字を抽出
    App->>Server: 不足フォントをサブセット要求
    Server-->>App: サブセットフォント
    App-->>User: レイアウト描画完了
```

### 3. ユースケース図（Use Case Diagram）

```mermaid
usecase-beta
direction LR
actor User("利用者")
actor Reviewer("監査担当者")
systemBoundary "Docs ブラウザアプリ"
  Open("文書を開く・編集")
  Export("PDF 出力")
  Audit("SBOM / 脆弱性確認")
end
User --> Open
User --> Export
Reviewer --> Audit
```

### 4. 状態遷移図（State Diagram）

```mermaid
stateDiagram-v2
    [*] --> StartView: アプリ起動
    StartView --> Loading: ファイルを開く
    Loading --> Editing: 読み込み完了
    Loading --> StartView: 読み込み失敗
    Editing --> Dirty: 編集操作
    Dirty --> Editing: 保存完了
    Editing --> StartView: 閉じる
    Dirty --> ConfirmSave: 閉じる
    ConfirmSave --> StartView: 保存または破棄
    ConfirmSave --> Dirty: キャンセル
```

### 5. アクティビティ／フローチャート（Flowchart）

```mermaid
flowchart LR
    A[文書を開く] --> B{形式を判定}
    B -->|DOCX / XLSX / PPTX| C[Office エディター]
    B -->|JTD / XDW / VSDX| D[固定レイアウト表示]
    B -->|Markdown| E[Markdown + Mermaid]
    B -->|SBOM / CBOM JSON| F[サプライチェーン表計算]
    C --> G[編集・PDF 出力]
    D --> G
    E --> G
    F --> G
```

### 6. スイムレーン図（Swimlanes）

```mermaid
swimlane-beta LR
  subgraph 開発者
    Build[リリースビルド]
    Bom[SBOM / CBOM 生成]
  end
  subgraph セキュリティ
    Scan[脆弱性スキャン]
    Triage[影響評価・優先度付け]
  end
  subgraph 運用
    Deploy[本番配信]
  end
  Build --> Bom
  Bom --> Scan
  Scan --> Triage
  Triage --> Deploy
```

### 7. 要求図（Requirement Diagram）

```mermaid
requirementDiagram

requirement wasm_sandbox {
  id: REQ-01
  text: 文書データはブラウザ内で完結して処理すること
  risk: high
  verifymethod: test
}

element docs_wasm {
  type: module
}

docs_wasm - satisfies -> wasm_sandbox
```

### 8. ZenUML（UML シーケンス記法）

```mermaid
zenuml
    title 文書オープン処理
    @Actor User
    @Boundary Editor
    @Database Storage
    User->Editor: openSample()
    Editor->Storage: fetchBytes()
    Storage->Editor: bytes
    Editor->User: render()
```

---

## 第 2 部：アーキテクチャ・データ・ネットワーク

### 9. ER 図（Entity Relationship Diagram）

```mermaid
erDiagram
    USER ||--o{ DOCUMENT : owns
    DOCUMENT ||--|{ VERSION : has
    DOCUMENT }o--o{ TAG : tagged_with

    USER {
        string id PK
        string name
    }
    DOCUMENT {
        string id PK
        string title
        string kind
    }
    VERSION {
        int rev PK
        string sha256
    }
    TAG {
        string name PK
    }
```

### 10. C4 コンテキスト図（C4 Context）

```mermaid
C4Context
  title Docs システムコンテキスト
  Person(user, "利用者", "ブラウザで文書を閲覧・編集")
  System(docs, "Docs (WASM)", "クライアント完結型オフィススイート")
  System_Ext(cdn, "静的配信 / フォント", "WASM チャンクとサブセットフォントを配信")
  Rel(user, docs, "文書を開く・編集")
  Rel(docs, cdn, "HTTPS でアセット取得")
```

### 11. アーキテクチャ図（Architecture）

```mermaid
architecture-beta
    group edge(cloud)[配信基盤]
    service worker(server)[Worker] in edge
    service assets(disk)[Static Assets] in edge
    service fonts(database)[Font Store] in edge

    worker:R -- L:assets
    worker:B -- T:fonts
```

### 12. ブロック図（Block Diagram）

```mermaid
block-beta
  columns 3
  a["UI Shell (GPUI)"] b["Doc Model"] c["Formats / Importers"]
  d["WASM Renderer"]:2 e["Font Subsetter"]
  a --> b
  b --> c
```

### 13. パケット図（Packet Diagram）

```mermaid
packet-beta
  0-15: "Magic / Version"
  16-31: "Flags"
  32-63: "Payload Length"
  64-95: "SHA-256 Digest (Prefix)"
```

---

## 第 3 部：計画・プロジェクト管理・思考整理

### 14. ガントチャート（Gantt Chart）

```mermaid
gantt
    title リリース準備スケジュール
    dateFormat YYYY-MM-DD
    section 設計・実装
        スタート画面改修    :done,    dev1, 2026-11-01, 4d
        サンプル拡充        :active,  dev2, after dev1, 3d
    section 検証・公開
        自動テスト・SBOM検証 :        tst1, after dev2, 2d
        本番デプロイ        :         rel1, after tst1, 1d
```

### 15. タイムライン（Timeline）

```mermaid
timeline
    title Docs 開発マイルストーン
    Phase 1 : Office 形式のブラウザ描画
            : WebGPU + WASM 基盤
    Phase 2 : 日本語動的フォント・一太郎対応
            : DocuWorks / Visio / Hancom 対応
    Phase 3 : SBOM / CBOM 検証と脆弱性レビュー
            : アプリ完結型スタート画面
```

### 16. カンバン（Kanban）

```mermaid
kanban
    todo[未着手]
        t1[BYOK AI 設定画面の洗練]
    doing[進行中]
        t2[スタート画面のカスタマイズ]
        t3[SBOM / 脆弱性サンプル拡充]
    done[完了]
        t4[WASM チャンク配信]
        t5[日本語動的フォント対応]
```

### 17. マインドマップ（Mindmap）

```mermaid
mindmap
  root((Docs))
    対応形式
      Word / Excel / PowerPoint
      一太郎 / DocuWorks / Visio
      Hancom / Markdown / CSV
      SBOM / CBOM JSON
    強み
      ブラウザ完結 (WASM)
      日本語縦書き・組版
      脆弱性レビュー支援
```

### 18. ユーザージャーニー（User Journey）

```mermaid
journey
    title サンプル確認から編集までの体験
    section 起動
      サイトを開く: 5: 利用者
      サンプル一覧を確認: 5: 利用者
    section 閲覧・編集
      WASM内で即時オープン: 5: 利用者
      閉じてスタート画面へ戻る: 5: 利用者
```

---

## 第 4 部：データ可視化・分析・開発

### 19. 円グラフ（Pie Chart）

```mermaid
pie showData
    title サンプル文書のカテゴリ構成
    "Office (DOCX/XLSX/PPTX)" : 5
    "国内・独自形式 (JTD/XDW/HWPX/VSDX)" : 4
    "構造化データ (SBOM/CBOM/CSV)" : 3
    "Markdown / Mermaid" : 1
```

### 20. XY チャート（XY Chart）

```mermaid
xychart-beta
    title "月別ドキュメント処理件数"
    x-axis [4月, 5月, 6月, 7月, 8月, 9月]
    y-axis "件数" 0 --> 120
    bar [42, 58, 65, 80, 94, 110]
    line [38, 52, 60, 75, 88, 104]
```

### 21. 四象限チャート（Quadrant Chart）

```mermaid
quadrantChart
    title 脆弱性トリアージ優先度マトリクス
    x-axis 到達性 低 --> 到達性 高
    y-axis 深刻度 低 --> 深刻度 高
    quadrant-1 最優先で即時対応
    quadrant-2 計画的に修正
    quadrant-3 監視継続
    quadrant-4 緩和策を検討
    CVE-Sample-01: [0.85, 0.90]
    CVE-Sample-02: [0.72, 0.78]
    CVE-Sample-03: [0.35, 0.65]
    CVE-Sample-04: [0.25, 0.30]
```

### 22. サンキー図（Sankey Diagram）

```mermaid
sankey
SBOM部品,安全な部品,680
SBOM部品,要確認部品,28
要確認部品,Critical/High,5
要確認部品,Medium/Low,23
Critical/High,修正版あり,4
Critical/High,到達性なし,1
```

### 23. レーダーチャート（Radar Chart）

```mermaid
radar-beta
  title ブラウザ完結オフィス環境の評価軸
  axis speed["起動速度"], jp["日本語組版"], formats["対応形式"], security["端末内完結"], audit["監査・BOM"]
  curve docs["Docs (WASM)"]{85, 92, 95, 98, 90}
  max 100
  min 0
```

### 24. Git グラフ（Git Graph）

```mermaid
gitGraph
    commit id: "init"
    branch feat/start-view
    checkout feat/start-view
    commit id: "start config"
    commit id: "mermaid 24 diagrams"
    commit id: "sbom/cbom samples"
    checkout main
    commit id: "hotfix"
    merge feat/start-view id: "integrate"
```
