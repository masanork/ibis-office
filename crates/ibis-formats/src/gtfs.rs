//! GTFS / GTFS-JP (標準的なバス情報フォーマット) ZIP パッケージの検査と Markdown サマリ出力。

use std::collections::{BTreeMap, HashMap};

use crate::Error;

const REQUIRED_FILES: &[&str] = &[
    "agency.txt",
    "stops.txt",
    "routes.txt",
    "trips.txt",
    "stop_times.txt",
];

const OPTIONAL_FILES: &[&str] = &[
    "calendar.txt",
    "calendar_dates.txt",
    "fare_attributes.txt",
    "fare_rules.txt",
    "shapes.txt",
    "frequencies.txt",
    "transfers.txt",
    "feed_info.txt",
    "translations.txt",
    "agency_jp.txt",
    "routes_jp.txt",
    "office_jp.txt",
    "pattern_jp.txt",
    "stop_times_jp.txt",
];

const GTFS_JP_FILES: &[&str] = &[
    "agency_jp.txt",
    "routes_jp.txt",
    "office_jp.txt",
    "pattern_jp.txt",
    "stop_times_jp.txt",
];

#[derive(Debug, Clone)]
pub struct GtfsFileStat {
    pub name: String,
    pub rows: usize,
    pub required: bool,
}

#[derive(Debug, Clone)]
pub struct GtfsRouteSummary {
    pub id: String,
    pub short_name: String,
    pub long_name: String,
    pub route_type: String,
}

#[derive(Debug, Clone)]
pub struct GtfsReport {
    pub is_gtfs_jp: bool,
    pub files: Vec<GtfsFileStat>,
    pub missing_required: Vec<String>,
    pub has_calendar: bool,
    pub agencies: Vec<String>,
    pub routes: Vec<GtfsRouteSummary>,
    pub feed_info: Vec<(String, String)>,
}

fn normalize_member_name(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

/// ZIP バイト列が GTFS / GTFS-JP パッケージかどうかを判定する。
pub fn is_gtfs(data: &[u8]) -> bool {
    if data.len() < 4 || (!data.starts_with(b"PK\x03\x04") && !data.starts_with(b"PK\x05\x06")) {
        return false;
    }
    let Ok(archive) = crate::zip::Archive::new(data) else {
        return false;
    };
    let mut has_agency = false;
    let mut core_hits = 0usize;
    for name in archive.names() {
        let base = normalize_member_name(&name);
        if base.eq_ignore_ascii_case("agency.txt") {
            has_agency = true;
            core_hits += 1;
        } else if REQUIRED_FILES.iter().any(|r| base.eq_ignore_ascii_case(r)) {
            core_hits += 1;
        }
    }
    has_agency && core_hits >= 2
}

/// GTFS / GTFS-JP ZIP パッケージを検証・集計する。
pub fn inspect_gtfs(data: &[u8]) -> Result<GtfsReport, Error> {
    let archive = crate::zip::Archive::new(data)?;
    let mut by_base: BTreeMap<String, String> = BTreeMap::new();
    for name in archive.names() {
        let base = normalize_member_name(&name);
        if !base.is_empty() && base.ends_with(".txt") {
            by_base
                .entry(base.to_ascii_lowercase())
                .or_insert_with(|| name.to_string());
        }
    }

    let mut missing_required = Vec::new();
    for &req in REQUIRED_FILES {
        if !by_base.contains_key(req) {
            missing_required.push(req.to_string());
        }
    }
    let has_calendar =
        by_base.contains_key("calendar.txt") || by_base.contains_key("calendar_dates.txt");

    let is_gtfs_jp = GTFS_JP_FILES.iter().any(|f| by_base.contains_key(*f));

    let mut files = Vec::new();
    let mut raw_contents: HashMap<String, String> = HashMap::new();

    for (base_lower, full_name) in &by_base {
        let bytes = archive
            .read(full_name)?
            .ok_or_else(|| Error::new(format!("missing archive entry: {full_name}")))?;
        let text = decode_utf8_or_sjis(&bytes);
        let rows = count_csv_data_rows(&text);
        let required = REQUIRED_FILES.contains(&base_lower.as_str());
        if matches!(
            base_lower.as_str(),
            "agency.txt" | "routes.txt" | "feed_info.txt"
        ) {
            raw_contents.insert(base_lower.clone(), text);
        }
        let known = required || OPTIONAL_FILES.contains(&base_lower.as_str());
        if known || rows > 0 {
            files.push(GtfsFileStat {
                name: base_lower.clone(),
                rows,
                required,
            });
        }
    }

    let agencies = raw_contents
        .get("agency.txt")
        .map(|t| extract_column_values(t, "agency_name"))
        .unwrap_or_default();

    let routes = raw_contents
        .get("routes.txt")
        .map(|t| extract_routes(t))
        .unwrap_or_default();

    let feed_info = raw_contents
        .get("feed_info.txt")
        .map(|t| extract_feed_info(t))
        .unwrap_or_default();

    Ok(GtfsReport {
        is_gtfs_jp,
        files,
        missing_required,
        has_calendar,
        agencies,
        routes,
        feed_info,
    })
}

/// GTFS / GTFS-JP パッケージの検証結果と概要を Markdown で返す。
pub fn to_markdown(data: &[u8]) -> Result<String, Error> {
    let report = inspect_gtfs(data)?;
    let mut out = String::new();
    let kind = if report.is_gtfs_jp {
        "GTFS-JP"
    } else {
        "GTFS"
    };
    out.push_str(&format!("# {kind} フィード検証サマリ\n\n"));

    if report.missing_required.is_empty() && report.has_calendar {
        out.push_str("- **必須ファイル検証**: OK (必須テーブル完備)\n");
    } else {
        let mut missing = report.missing_required.clone();
        if !report.has_calendar {
            missing.push("calendar.txt / calendar_dates.txt".to_string());
        }
        out.push_str(&format!(
            "- **必須ファイル検証**: 不足あり (`{}`)\n",
            missing.join("`, `")
        ));
    }

    if !report.agencies.is_empty() {
        out.push_str(&format!(
            "- **事業者 (`agency_name`)**: {}\n",
            report.agencies.join(" / ")
        ));
    }
    for (k, v) in &report.feed_info {
        out.push_str(&format!("- **{k}**: {v}\n"));
    }

    out.push_str("\n## 収録テーブル一覧\n\n");
    out.push_str("| ファイル名 | 種別 | レコード数 |\n|---|---|---:|\n");
    for f in &report.files {
        let kind_label = if f.required {
            "必須"
        } else if GTFS_JP_FILES.contains(&f.name.as_str()) {
            "GTFS-JP"
        } else {
            "任意"
        };
        out.push_str(&format!("| `{}` | {} | {} |\n", f.name, kind_label, f.rows));
    }

    if !report.routes.is_empty() {
        out.push_str("\n## 路線一覧 (先頭最大 30 件)\n\n");
        out.push_str("| route_id | 路線略称 | 路線名 | 種別 |\n|---|---|---|---|\n");
        for r in report.routes.iter().take(30) {
            out.push_str(&format!(
                "| `{}` | {} | {} | {} |\n",
                escape_md(&r.id),
                escape_md(&r.short_name),
                escape_md(&r.long_name),
                escape_md(&r.route_type)
            ));
        }
        if report.routes.len() > 30 {
            out.push_str(&format!("\n*…他 {} 路線*\n", report.routes.len() - 30));
        }
    }

    Ok(out)
}

fn escape_md(s: &str) -> String {
    s.replace('|', "\\|").replace(['\r', '\n'], " ")
}

fn decode_utf8_or_sjis(bytes: &[u8]) -> String {
    let stripped = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if let Ok(s) = std::str::from_utf8(stripped) {
        return s.to_string();
    }
    let (cow, _, _) = encoding_rs::SHIFT_JIS.decode(stripped);
    cow.into_owned()
}

fn parse_csv_rows(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(c);
            }
        } else {
            match c {
                '"' => in_quotes = true,
                ',' => {
                    row.push(std::mem::take(&mut field));
                }
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    row.push(std::mem::take(&mut field));
                    if row.iter().any(|f| !f.trim().is_empty()) {
                        rows.push(std::mem::take(&mut row));
                    } else {
                        row.clear();
                    }
                }
                '\n' => {
                    row.push(std::mem::take(&mut field));
                    if row.iter().any(|f| !f.trim().is_empty()) {
                        rows.push(std::mem::take(&mut row));
                    } else {
                        row.clear();
                    }
                }
                _ => field.push(c),
            }
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        if row.iter().any(|f| !f.trim().is_empty()) {
            rows.push(row);
        }
    }
    rows
}

fn count_csv_data_rows(text: &str) -> usize {
    let rows = parse_csv_rows(text);
    rows.len().saturating_sub(1)
}

fn header_index<'a>(headers: &'a [String], key: &str) -> Option<usize> {
    headers
        .iter()
        .position(|h| h.trim().eq_ignore_ascii_case(key))
}

fn extract_column_values(text: &str, col: &str) -> Vec<String> {
    let rows = parse_csv_rows(text);
    let Some(headers) = rows.first() else {
        return Vec::new();
    };
    let Some(idx) = header_index(headers, col) else {
        return Vec::new();
    };
    rows.iter()
        .skip(1)
        .filter_map(|r| r.get(idx).map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .collect()
}

fn extract_routes(text: &str) -> Vec<GtfsRouteSummary> {
    let rows = parse_csv_rows(text);
    let Some(headers) = rows.first() else {
        return Vec::new();
    };
    let idx_id = header_index(headers, "route_id");
    let idx_short = header_index(headers, "route_short_name");
    let idx_long = header_index(headers, "route_long_name");
    let idx_type = header_index(headers, "route_type");

    rows.iter()
        .skip(1)
        .map(|r| {
            let get = |idx: Option<usize>| {
                idx.and_then(|i| r.get(i))
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default()
            };
            GtfsRouteSummary {
                id: get(idx_id),
                short_name: get(idx_short),
                long_name: get(idx_long),
                route_type: get(idx_type),
            }
        })
        .collect()
}

fn extract_feed_info(text: &str) -> Vec<(String, String)> {
    let rows = parse_csv_rows(text);
    let (Some(headers), Some(first)) = (rows.first(), rows.get(1)) else {
        return Vec::new();
    };
    let mut info = Vec::new();
    for (label, key) in [
        ("提供者 (`feed_publisher_name`)", "feed_publisher_name"),
        ("バージョン (`feed_version`)", "feed_version"),
        ("有効開始日 (`feed_start_date`)", "feed_start_date"),
        ("有効終了日 (`feed_end_date`)", "feed_end_date"),
    ] {
        if let Some(idx) = header_index(headers, key) {
            if let Some(val) = first.get(idx).map(|s| s.trim()).filter(|s| !s.is_empty()) {
                info.push((label.to_string(), val.to_string()));
            }
        }
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &b in data {
            crc ^= b as u32;
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
        !crc
    }

    fn make_stored_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for &(name, data) in entries {
            let offset = out.len() as u32;
            let crc = crc32(data);
            let size = data.len() as u32;
            let name_bytes = name.as_bytes();
            let name_len = name_bytes.len() as u16;

            out.extend_from_slice(b"PK\x03\x04");
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&name_len.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name_bytes);
            out.extend_from_slice(data);

            central.extend_from_slice(b"PK\x01\x02");
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&size.to_le_bytes());
            central.extend_from_slice(&size.to_le_bytes());
            central.extend_from_slice(&name_len.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u32.to_le_bytes());
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name_bytes);
        }
        let cd_offset = out.len() as u32;
        let cd_size = central.len() as u32;
        let count = entries.len() as u16;
        out.extend_from_slice(&central);
        out.extend_from_slice(b"PK\x05\x06");
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    #[test]
    fn validates_gtfs_jp_zip_and_generates_markdown() {
        let zip = make_stored_zip(&[
            (
                "agency.txt",
                "agency_id,agency_name,agency_url,agency_timezone\nA1,都営バス,https://example.jp,Asia/Tokyo\n".as_bytes(),
            ),
            (
                "stops.txt",
                "stop_id,stop_name,stop_lat,stop_lon\nS1,東京駅,35.6812,139.7671\n".as_bytes(),
            ),
            (
                "routes.txt",
                "route_id,agency_id,route_short_name,route_long_name,route_type\nR1,A1,都01,渋谷〜新橋,3\n".as_bytes(),
            ),
            (
                "trips.txt",
                "route_id,service_id,trip_id\nR1,WD,T1\n".as_bytes(),
            ),
            (
                "stop_times.txt",
                "trip_id,arrival_time,departure_time,stop_id,stop_sequence\nT1,08:00:00,08:00:00,S1,1\n".as_bytes(),
            ),
            (
                "calendar.txt",
                "service_id,monday,tuesday,wednesday,thursday,friday,saturday,sunday,start_date,end_date\nWD,1,1,1,1,1,0,0,20260101,20261231\n".as_bytes(),
            ),
            (
                "routes_jp.txt",
                "route_id,route_update_date,origin_stop,via_stop,destination_stop\nR1,20260101,渋谷,六本木,新橋\n".as_bytes(),
            ),
            (
                "feed_info.txt",
                "feed_publisher_name,feed_publisher_url,feed_lang,feed_version\n東京都交通局,https://example.jp,ja,2026.1\n".as_bytes(),
            ),
        ]);

        assert!(is_gtfs(&zip));
        let report = inspect_gtfs(&zip).unwrap();
        assert!(report.is_gtfs_jp);
        assert!(report.missing_required.is_empty());
        assert!(report.has_calendar);
        assert_eq!(report.agencies, vec!["都営バス".to_string()]);
        assert_eq!(report.routes.len(), 1);

        let md = to_markdown(&zip).unwrap();
        assert!(md.contains("GTFS-JP フィード検証サマリ"));
        assert!(md.contains("都営バス"));
        assert!(md.contains("渋谷〜新橋"));
        assert!(md.contains("routes_jp.txt"));
    }

    #[test]
    fn reports_missing_required_tables() {
        let zip = make_stored_zip(&[
            (
                "agency.txt",
                "agency_id,agency_name\nA1,コミュニティバス\n".as_bytes(),
            ),
            (
                "stops.txt",
                "stop_id,stop_name\nS1,市役所前\n".as_bytes(),
            ),
        ]);

        assert!(is_gtfs(&zip));
        let report = inspect_gtfs(&zip).unwrap();
        assert!(!report.is_gtfs_jp);
        assert!(report.missing_required.contains(&"routes.txt".to_string()));
        assert!(!report.has_calendar);
    }
}
