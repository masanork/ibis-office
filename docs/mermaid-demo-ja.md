# Mermaid 図種別デモ

Docs でこの Markdown を開くと、図種別の表示を確認できます。[Markdown Reference](markdown-reference-ja.md) には対応する構文・設定と、数値 x 軸や横棒グラフなどの追加例があります。

このページは固定した Mermaid 公式コーパスから、図種ごとに1例を選んだものです。例の出典とライセンスは [SOURCE.json](../crates/ibis-formats/tests/mermaid/corpus/SOURCE.json) と [MIT ライセンス](../crates/ibis-formats/tests/mermaid/corpus/LICENSE) を参照してください。全例を図種別の Markdown に生成する手順は [Reference](markdown-reference-ja.md) にあります。

## エージェントフロー

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/agentflow.md) · [コーパス例 11](../crates/ibis-formats/tests/mermaid/corpus/agentflow.mmd)

```mermaid
---
config:
  agentflow:
    nodeSpacing: 70
    rankSpacing: 70
---
agentflow-beta TB
  a["A"] --> b["B"]
```

同じ段のノード同士は `nodeSpacing`、段と段の間は `rankSpacing` で調整します。指定できる範囲は 0〜1000 px です。矢印のラベルがあるときは、そのラベルに必要な間隔を確保します。

```mermaid
---
config:
  agentflow:
    nodeSpacing: 100
    rankSpacing: 120
---
agentflow TB
  a["受付"] --> b["審査"]
  a --> c["記録"]
```

## アーキテクチャ

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/architecture.md) · [コーパス例 3](../crates/ibis-formats/tests/mermaid/corpus/architecture.mmd)

```mermaid
architecture-beta
    service src1(server)[Source 1]
    service src2(server)[Source 2]
    service src3(server)[Source 3]
    service proc(server)[Processor]
    src1:B --> T:proc
    src2:B --> T:proc
    src3:B --> T:proc
    align row src1 src2 src3
```

## ブロック

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/block.md) · [コーパス例 2](../crates/ibis-formats/tests/mermaid/corpus/block.mmd)

```mermaid
block
  a b c
```

## C4

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/c4.md) · [コーパス例 4](../crates/ibis-formats/tests/mermaid/corpus/c4.mmd)

```mermaid
    C4Dynamic
    title Dynamic diagram for Internet Banking System - API Application

    ContainerDb(c4, "Database", "Relational Database Schema", "Stores user registration information, hashed authentication credentials, access logs, etc.")
    Container(c1, "Single-Page Application", "JavaScript and Angular", "Provides all of the Internet banking functionality to customers via their web browser.")
    Container_Boundary(b, "API Application") {
      Component(c3, "Security Component", "Spring Bean", "Provides functionality Related to signing in, changing passwords, etc.")
      Component(c2, "Sign In Controller", "Spring MVC Rest Controller", "Allows users to sign in to the Internet Banking System.")
    }
    Rel(c1, c2, "Submits credentials to", "JSON/HTTPS")
    Rel(c2, c3, "Calls isAuthenticated() on")
    Rel(c3, c4, "select * from users where username = ?", "JDBC")

    UpdateRelStyle(c1, c2, $textColor="red", $offsetY="-40")
    UpdateRelStyle(c2, c3, $textColor="red", $offsetX="-40", $offsetY="60")
    UpdateRelStyle(c3, c4, $textColor="red", $offsetY="-40", $offsetX="10")
```

## クラス

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/classDiagram.md) · [コーパス例 16](../crates/ibis-formats/tests/mermaid/corpus/classDiagram.mmd)

```mermaid
classDiagram
  bar ()-- foo
```

## ER

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/entityRelationshipDiagram.md) · [コーパス例 5](../crates/ibis-formats/tests/mermaid/corpus/entityRelationshipDiagram.mmd)

```mermaid
erDiagram
    "This ❤ Unicode"
```

## イベントモデリング

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/eventmodeling.md) · [コーパス例 1](../crates/ibis-formats/tests/mermaid/corpus/eventmodeling.mmd)

```mermaid
eventmodeling

tf 01 ui CartUI
tf 02 cmd AddItem
tf 03 evt ItemAdded
```

## フローチャート

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/flowchart.md) · [コーパス例 71](../crates/ibis-formats/tests/mermaid/corpus/flowchart.mmd)

```mermaid
flowchart LR
    A-->B
```

## ガント

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/gantt.md) · [コーパス例 7](../crates/ibis-formats/tests/mermaid/corpus/gantt.mmd)

```mermaid
gantt
  tickInterval 1week
  weekday monday
```

## Gitグラフ

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/gitgraph.md) · [コーパス例 2](../crates/ibis-formats/tests/mermaid/corpus/gitgraph.mmd)

```mermaid
    gitGraph
       commit
       commit
       commit
```

## カンバン

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/kanban.md) · [コーパス例 1](../crates/ibis-formats/tests/mermaid/corpus/kanban.mmd)

```mermaid
kanban
  column1[Column Title]
    task1[Task Description]
```

## マインドマップ

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/mindmap.md) · [コーパス例 7](../crates/ibis-formats/tests/mermaid/corpus/mindmap.mmd)

```mermaid
mindmap
    id)I am a cloud(
```

## パケット

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/packet.md) · [コーパス例 2](../crates/ibis-formats/tests/mermaid/corpus/packet.mmd)

```mermaid
packet
title UDP Packet
+16: "Source Port"
+16: "Destination Port"
32-47: "Length"
48-63: "Checksum"
64-95: "Data (variable length)"
```

## 円グラフ

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/pie.md) · [コーパス例 1](../crates/ibis-formats/tests/mermaid/corpus/pie.mmd)

```mermaid
pie title Pets adopted by volunteers
    "Dogs" : 386
    "Cats" : 85
    "Rats" : 15
```

## 四象限

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/quadrantChart.md) · [コーパス例 2](../crates/ibis-formats/tests/mermaid/corpus/quadrantChart.mmd)

```mermaid
---
config:
  quadrantChart:
    chartWidth: 400
    chartHeight: 400
  themeVariables:
    quadrant1TextFill: "ff0000"
---
quadrantChart
  x-axis Urgent --> Not Urgent
  y-axis Not Important --> "Important ❤"
  quadrant-1 Plan
  quadrant-2 Do
  quadrant-3 Delegate
  quadrant-4 Delete
```

## レーダー

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/radar.md) · [コーパス例 1](../crates/ibis-formats/tests/mermaid/corpus/radar.mmd)

```mermaid
---
title: "Grades"
---
radar-beta
  axis m["Math"], s["Science"], e["English"]
  axis h["History"], g["Geography"], a["Art"]
  curve a["Alice"]{85, 90, 80, 70, 75, 90}
  curve b["Bob"]{70, 75, 85, 80, 90, 85}

  max 100
  min 0
```

## 構文図

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/railroad.md) · [コーパス例 4](../crates/ibis-formats/tests/mermaid/corpus/railroad.mmd)

```mermaid
railroad-ebnf-beta
sign = "+" | "-" ;
```

## 要求

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/requirementDiagram.md) · [コーパス例 4](../crates/ibis-formats/tests/mermaid/corpus/requirementDiagram.mmd)

```mermaid
requirementDiagram

requirement "__test_req__" {
    id: 1
    text: "*italicized text* **bold text**"
    risk: high
    verifymethod: test
}
```

## サンキー

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/sankey.md) · [コーパス例 3](../crates/ibis-formats/tests/mermaid/corpus/sankey.mmd)

```mermaid
sankey

Bio-conversion,Losses,26.862

Bio-conversion,Solid,280.322

Bio-conversion,Gas,81.144
```

## シーケンス

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/sequenceDiagram.md) · [コーパス例 22](../crates/ibis-formats/tests/mermaid/corpus/sequenceDiagram.mmd)

```mermaid
sequenceDiagram
    participant John
    Note right of John: Text in note
```

## 状態遷移

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/stateDiagram.md) · [コーパス例 5](../crates/ibis-formats/tests/mermaid/corpus/stateDiagram.mmd)

```mermaid
stateDiagram-v2
    stateId
```

## スイムレーン

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/swimlanes.md) · [コーパス例 4](../crates/ibis-formats/tests/mermaid/corpus/swimlanes.mmd)

```mermaid
swimlane-beta
  subgraph Sales
    lead[Qualify lead]
    quote[Prepare quote]
  end
```

## タイムライン

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/timeline.md) · [コーパス例 2](../crates/ibis-formats/tests/mermaid/corpus/timeline.mmd)

```mermaid
timeline
    title History of Social Media Platform
    2002 : LinkedIn
    2004 : Facebook : Google
    2005 : YouTube
    2006 : Twitter
```

## ツリー

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/treeView.md) · [コーパス例 11](../crates/ibis-formats/tests/mermaid/corpus/treeView.mmd)

```mermaid
treeView-beta
    "packages"
        "mermaid"
            "src"
        "parser"
```

## ツリーマップ

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/treemap.md) · [コーパス例 1](../crates/ibis-formats/tests/mermaid/corpus/treemap.mmd)

```mermaid
treemap-beta
"Category A"
    "Item A1": 10
    "Item A2": 20
"Category B"
    "Item B1": 15
    "Item B2": 25
```

## ユースケース

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/usecase.md) · [コーパス例 13](../crates/ibis-formats/tests/mermaid/corpus/usecase.mmd)

```mermaid
usecase-beta
actor User
Login
User -- "include account details" --> Login
```

## ユーザージャーニー

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/userJourney.md) · [コーパス例 1](../crates/ibis-formats/tests/mermaid/corpus/userJourney.mmd)

```mermaid
journey
    title My working day
    section Go to work
      Make tea: 5: Me
      Go upstairs: 3: Me
      Do work: 1: Me, Cat
    section Go home
      Go downstairs: 5: Me
      Sit down: 5: Me
```

## ベン図

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/venn.md) · [コーパス例 4](../crates/ibis-formats/tests/mermaid/corpus/venn.mmd)

```mermaid
venn-beta
  set A["Alpha"]
  set B["Beta"]
  union A,B["AB"]
```

## Wardley

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/wardley.md) · [コーパス例 18](../crates/ibis-formats/tests/mermaid/corpus/wardley.mmd)

```mermaid
wardley-beta
title Custom Size
size [800, 1000]
```

## XYグラフ

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/xyChart.md) · [コーパス例 7](../crates/ibis-formats/tests/mermaid/corpus/xyChart.mmd)

```mermaid
xychart
    title "Quarterly Performance"
    x-axis [Q1, Q2, Q3, Q4]
    y-axis "Revenue ($M)" 0 --> 100
    line [25 "Launch", 45, 72, 90 "Target Hit"]
```

## 公式コーパスに単独例がない構文

### 共通テーマ変数とリンクの装飾

共通テーマ変数と `linkStyle` で、ノードと矢印に別の色を指定します。

```mermaid
---
config:
  themeVariables:
    primaryColor: "#dbeafe"
    lineColor: "#334155"
    background: "#ffffff"
---
flowchart LR
  A[受付] --> B[承認]
  linkStyle 0 stroke:#2563eb,stroke-width:3px
```

### カンバンの列幅

```mermaid
---
config:
  kanban:
    sectionWidth: 250
---
kanban
  todo[確認待ち]
    item[申請書]@{ priority: 'High' }
  done[確認済み]
    reviewed[添付資料]
```

双方向シーケンス矢印、横向き、数値 x 軸は [Markdown Reference](markdown-reference-ja.md) の追加例で確認できます。

## 対応範囲の比較

### 初期化ディレクティブ

`%%{init}%%` の指定は未対応です。上の例のように YAML の `config` を使用できます。

```mermaid
%%{init: {"theme": "forest"}}%%
flowchart LR
  A[受付] --> B[承認]
```

### ガントの todayMarker

図の生成時点のローカル日時が時間軸の範囲内なら、当日の線を描きます。線の色・太さ・不透明度を指定できます。

```mermaid
gantt
  dateFormat YYYY-MM-DD
  tickInterval 1year
  todayMarker stroke-width:3px,stroke:#2563eb,opacity:0.7
  section 審査
  実施期間 :2025-01-01, 2030-01-01
```

`todayMarker off` を指定すると、時間軸の範囲内でも線を非表示にできます。

```mermaid
gantt
  dateFormat YYYY-MM-DD
  tickInterval 1year
  todayMarker off
  section 審査
  実施期間 :2025-01-01, 2030-01-01
```

### 円グラフの凡例位置

凡例は右・左・上・下・中央に配置できます。省略時は右に表示します。中央の凡例は円グラフに重なり、図全体を SVG として表示します。

**下**

```mermaid
---
config:
  pie:
    legendPosition: bottom
---
pie
  "了承": 80
  "保留": 20
```

**上**

```mermaid
---
config:
  pie:
    legendPosition: top
---
pie
  "了承": 80
  "保留": 20
```

**左**

```mermaid
---
config:
  pie:
    legendPosition: left
---
pie
  "了承": 80
  "保留": 20
```

**右**

```mermaid
---
config:
  pie:
    legendPosition: right
---
pie
  "了承": 80
  "保留": 20
```

**中央**

```mermaid
---
config:
  pie:
    legendPosition: center
---
pie
  "了承": 80
  "保留": 20
```

## Cynefin

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/cynefin.md) · [コーパス例 3](../crates/ibis-formats/tests/mermaid/corpus/cynefin.mmd)

```mermaid
cynefin-beta
  title Cynefin Framework

  complex
  complicated
  clear
    "既知の手順で対応"
  chaotic
  confusion
    "原因を調査中"
  clear --> chaotic : "状況が急変"
```

## 特性要因図（Ishikawa）

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/ishikawa.md) · [コーパス例 1](../crates/ibis-formats/tests/mermaid/corpus/ishikawa.mmd)

```mermaid
ishikawa-beta
    Blurry Photo
    Process
        Out of focus
        Shutter speed too slow
        Protective film not removed
        Beautification filter applied
    User
        Shaky hands
    Equipment
        LENS
            Inappropriate lens
            Damaged lens
            Dirty lens
        SENSOR
            Damaged sensor
            Dirty sensor
    Environment
        Subject moved too quickly
        Too dark
```

## ZenUML

[公式構文](https://github.com/mermaid-js/mermaid/blob/2086904e76c692331e34a45d7cc3bc30860d040e/docs/syntax/zenuml.md) · [コーパス例 7](../crates/ibis-formats/tests/mermaid/corpus/zenuml.mmd)

```mermaid
zenuml
    new A1
    new A2(with, parameters)
```
