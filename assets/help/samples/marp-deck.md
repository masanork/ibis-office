---
marp: true
theme: default
paginate: true
---

# soraneDocs Marp デモ

ブラウザ上で **Source ↔ スライド編集** をリアルタイム同期します。

---

## 双方向 Split 編集

- 左のソースを編集するとスライドが即時更新
- 右のスライドでテキストや図形を編集すると Markdown へ反映
- `---` 単位でアクティブスライドと行範囲が連動

```mermaid
flowchart LR
  Source["Marp Markdown"] --> Visual["スライド編集"]
  Visual --> Source
```

---

## まとめ

1. YAML フロントマター（`marp: true` / `theme` / `paginate`）を保持
2. Mermaid 図もブラウザ内で自動レンダリング
3. `.md` としてそのまま保存・GitHub コミット可能
