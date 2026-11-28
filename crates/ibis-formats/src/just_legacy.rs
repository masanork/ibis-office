//! ジャストシステム 花子 (`.jhd`/`.jhb`)・三四郎 (`.jsd`) レガシー OLE2 (CFB) 形式の判定と Markdown サマリ出力。

use ibis_ooxml::cfb;

use crate::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JustLegacyKind {
    Hanako,
    Sanshiro,
}

impl JustLegacyKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Hanako => "ジャストシステム 花子 (.jhd/.jhb)",
            Self::Sanshiro => "ジャストシステム 三四郎 (.jsd)",
        }
    }
}

#[derive(Debug, Clone)]
pub struct JustStreamInfo {
    pub path: String,
    pub is_storage: bool,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct JustLegacyReport {
    pub kind: JustLegacyKind,
    pub streams: Vec<JustStreamInfo>,
    pub notes: Vec<String>,
}

fn detect_kind_from_entries(data: &[u8], entries: &[cfb::DirectoryEntry]) -> Option<JustLegacyKind> {
    let mut has_hanako = false;
    let mut has_sanshiro = false;

    for e in entries {
        let lower = e.path.to_ascii_lowercase();
        if lower.contains("hanako")
            || lower.contains("hnkbody")
            || lower.contains("hnkinfo")
            || lower.contains("jhdinfo")
            || lower.contains("jhbinfo")
        {
            has_hanako = true;
        }
        if lower.contains("sanshiro")
            || lower.contains("jsdinfo")
            || lower.contains("snsinfo")
            || lower.contains("snsbook")
        {
            has_sanshiro = true;
        }
    }

    if has_hanako {
        return Some(JustLegacyKind::Hanako);
    }
    if has_sanshiro {
        return Some(JustLegacyKind::Sanshiro);
    }

    if let Ok(summary) = cfb::stream(data, "\u{5}SummaryInformation") {
        let text = decode_lossy_ascii_or_sjis(&summary);
        if text.contains("Hanako") || text.contains("花子") {
            return Some(JustLegacyKind::Hanako);
        }
        if text.contains("Sanshiro") || text.contains("三四郎") {
            return Some(JustLegacyKind::Sanshiro);
        }
    }

    None
}

/// バイト列が花子 (`.jhd`/`.jhb`) または三四郎 (`.jsd`) の CFB コンテナかどうかを判定する。
pub fn is_just_legacy(data: &[u8]) -> bool {
    if !cfb::is_cfb(data) {
        return false;
    }
    let Ok(entries) = cfb::directory(data) else {
        return false;
    };
    detect_kind_from_entries(data, &entries).is_some()
}

/// 花子 / 三四郎 CFB コンテナの構造を検査する。
pub fn inspect_just_legacy(data: &[u8]) -> Result<JustLegacyReport, Error> {
    if !cfb::is_cfb(data) {
        return Err(Error::new("not an OLE2 compound file"));
    }
    let reader = cfb::Reader::new(data).map_err(|e| Error::new(format!("CFB read error: {e}")))?;
    let entries = reader
        .directory()
        .map_err(|e| Error::new(format!("CFB directory error: {e}")))?;

    let kind = detect_kind_from_entries(data, entries)
        .ok_or_else(|| Error::new("not a JustSystem Hanako/Sanshiro CFB file"))?;

    let mut streams = Vec::with_capacity(entries.len());
    for entry in entries {
        let size = if entry.is_storage {
            0
        } else {
            reader.stream_size(&entry.path).unwrap_or(0)
        };
        streams.push(JustStreamInfo {
            path: entry.path.clone(),
            is_storage: entry.is_storage,
            size,
        });
    }

    let mut notes = Vec::new();
    notes.push(
        "独自バイナリストリーム形式のため、現在はコンテナ構造・ストリーム一覧の検査に対応しています。"
            .to_string(),
    );

    Ok(JustLegacyReport {
        kind,
        streams,
        notes,
    })
}

/// 花子 / 三四郎 CFB コンテナの検査結果を Markdown で返す。
pub fn to_markdown(data: &[u8]) -> Result<String, Error> {
    let report = inspect_just_legacy(data)?;
    let mut out = String::new();
    out.push_str(&format!("# {} 検査サマリ\n\n", report.kind.label()));
    out.push_str(&format!("- **種別**: {}\n", report.kind.label()));
    out.push_str(&format!("- **検出エントリ数**: {}\n", report.streams.len()));
    for note in &report.notes {
        out.push_str(&format!("- **注記**: {note}\n"));
    }

    out.push_str("\n## CFB ストリーム / ストレージ一覧\n\n");
    out.push_str("| パス | 種別 | サイズ (bytes) |\n|---|---|---:|\n");
    for s in &report.streams {
        let kind_label = if s.is_storage { "Storage" } else { "Stream" };
        let clean_path = s.path.replace(['\x05', '|', '\r', '\n'], " ");
        out.push_str(&format!("| `{}` | {} | {} |\n", clean_path, kind_label, s.size));
    }

    Ok(out)
}

fn decode_lossy_ascii_or_sjis(bytes: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    let (cow, _, _) = encoding_rs::SHIFT_JIS.decode(bytes);
    cow.into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_hanako_cfb_and_renders_markdown() {
        let cfb_bytes = cfb::write(&[
            cfb::Item::stream("Hanako", b"HNK1".to_vec()),
            cfb::Item::stream("HnkBody", vec![0u8; 128]),
        ]);
        assert!(is_just_legacy(&cfb_bytes));
        let report = inspect_just_legacy(&cfb_bytes).unwrap();
        assert_eq!(report.kind, JustLegacyKind::Hanako);
        let md = to_markdown(&cfb_bytes).unwrap();
        assert!(md.contains("花子"));
        assert!(md.contains("HnkBody"));
        assert!(md.contains("128"));
    }

    #[test]
    fn detects_sanshiro_cfb_and_renders_markdown() {
        let cfb_bytes = cfb::write(&[
            cfb::Item::stream("Sanshiro", b"SNS1".to_vec()),
            cfb::Item::stream("SnsBook", vec![1u8; 64]),
        ]);
        assert!(is_just_legacy(&cfb_bytes));
        let report = inspect_just_legacy(&cfb_bytes).unwrap();
        assert_eq!(report.kind, JustLegacyKind::Sanshiro);
        let md = to_markdown(&cfb_bytes).unwrap();
        assert!(md.contains("三四郎"));
        assert!(md.contains("SnsBook"));
    }
}
