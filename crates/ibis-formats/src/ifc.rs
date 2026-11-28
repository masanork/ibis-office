//! BIM/CIM IFC (ISO 10303-21 STEP Physical File) inspector and Markdown exporter.

use std::collections::BTreeMap;

use crate::Error;

/// Summary of an IFC (ISO 10303-21) BIM/CIM model.
#[derive(Debug, Clone, PartialEq)]
pub struct IfcSummary {
    pub schema: String,
    pub description: String,
    pub file_name: String,
    pub timestamp: String,
    pub authors: Vec<String>,
    pub organizations: Vec<String>,
    pub originating_system: String,
    pub total_entities: usize,
    pub projects: Vec<String>,
    pub sites: Vec<String>,
    pub buildings: Vec<String>,
    pub storeys: Vec<(String, Option<f64>)>,
    pub entity_counts: Vec<(String, usize)>,
}

/// Returns true when `data` looks like an IFC STEP Physical File (`ISO-10303-21;`).
pub fn is_ifc(data: &[u8]) -> bool {
    let head_len = data.len().min(4096);
    let head = String::from_utf8_lossy(&data[..head_len]);
    let upper = head.to_ascii_uppercase();
    upper.contains("ISO-10303-21;")
        && (upper.contains("'IFC")
            || upper.contains("IFC2X3")
            || upper.contains("IFC4")
            || upper.contains("IFCPROJECT"))
}

/// Inspects an IFC STEP file and extracts header metadata, spatial hierarchy, and entity counts.
pub fn inspect_ifc(data: &[u8]) -> Result<IfcSummary, Error> {
    if !is_ifc(data) {
        return Err(Error::new("Not an IFC STEP Physical File"));
    }

    let text = String::from_utf8_lossy(data);
    let mut summary = IfcSummary {
        schema: String::new(),
        description: String::new(),
        file_name: String::new(),
        timestamp: String::new(),
        authors: Vec::new(),
        organizations: Vec::new(),
        originating_system: String::new(),
        total_entities: 0,
        projects: Vec::new(),
        sites: Vec::new(),
        buildings: Vec::new(),
        storeys: Vec::new(),
        entity_counts: Vec::new(),
    };

    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut in_data = false;

    for raw_stmt in split_step_statements(&text) {
        let stmt = raw_stmt.trim();
        if stmt.is_empty() {
            continue;
        }
        let upper = stmt.to_ascii_uppercase();
        if upper == "DATA" {
            in_data = true;
            continue;
        }
        if upper == "ENDSEC" || upper == "HEADER" || upper.starts_with("ISO-10303-21") || upper.starts_with("END-ISO-10303-21") {
            if upper == "ENDSEC" {
                in_data = false;
            }
            continue;
        }

        if !in_data {
            if let Some(args_str) = extract_call_args(stmt, "FILE_DESCRIPTION") {
                let args = split_top_level_args(args_str);
                if let Some(first) = args.first() {
                    let items = extract_string_list(first);
                    if !items.is_empty() {
                        summary.description = items.join(", ");
                    }
                }
            } else if let Some(args_str) = extract_call_args(stmt, "FILE_SCHEMA") {
                let args = split_top_level_args(args_str);
                if let Some(first) = args.first() {
                    let items = extract_string_list(first);
                    if !items.is_empty() {
                        summary.schema = items.join(", ");
                    }
                }
            } else if let Some(args_str) = extract_call_args(stmt, "FILE_NAME") {
                let args = split_top_level_args(args_str);
                if let Some(s) = args.first().and_then(|a| parse_step_string(a)) {
                    summary.file_name = s;
                }
                if let Some(s) = args.get(1).and_then(|a| parse_step_string(a)) {
                    summary.timestamp = s;
                }
                if let Some(a) = args.get(2) {
                    summary.authors = extract_string_list(a);
                }
                if let Some(a) = args.get(3) {
                    summary.organizations = extract_string_list(a);
                }
                if let Some(s) = args.get(5).and_then(|a| parse_step_string(a)) {
                    summary.originating_system = s;
                }
            }
            continue;
        }

        // DATA section: `#123 = IFCTYPE(...);`
        if let Some((entity_name, args_str)) = parse_entity_instance(stmt) {
            summary.total_entities += 1;
            *counts.entry(entity_name.clone()).or_insert(0) += 1;

            match entity_name.as_str() {
                "IFCPROJECT" => {
                    let args = split_top_level_args(args_str);
                    if let Some(name) = args.get(2).and_then(|a| parse_step_string(a)) {
                        if !name.is_empty() {
                            summary.projects.push(name);
                        }
                    }
                }
                "IFCSITE" => {
                    let args = split_top_level_args(args_str);
                    if let Some(name) = args.get(2).and_then(|a| parse_step_string(a)) {
                        if !name.is_empty() {
                            summary.sites.push(name);
                        }
                    }
                }
                "IFCBUILDING" => {
                    let args = split_top_level_args(args_str);
                    if let Some(name) = args.get(2).and_then(|a| parse_step_string(a)) {
                        if !name.is_empty() {
                            summary.buildings.push(name);
                        }
                    }
                }
                "IFCBUILDINGSTOREY" => {
                    let args = split_top_level_args(args_str);
                    let name = args
                        .get(2)
                        .and_then(|a| parse_step_string(a))
                        .filter(|s| !s.is_empty())
                        .or_else(|| args.get(7).and_then(|a| parse_step_string(a)))
                        .unwrap_or_else(|| "(名称なし)".to_string());
                    let elevation = args.get(9).and_then(|a| a.trim().parse::<f64>().ok());
                    summary.storeys.push((name, elevation));
                }
                _ => {}
            }
        }
    }

    let mut entity_counts: Vec<(String, usize)> = counts.into_iter().collect();
    entity_counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    summary.entity_counts = entity_counts;

    Ok(summary)
}

/// Renders an IFC STEP file as a structured Markdown summary.
pub fn to_markdown(data: &[u8]) -> Result<String, Error> {
    let summary = inspect_ifc(data)?;
    let mut out = String::new();

    if let Some(proj) = summary.projects.first() {
        out.push_str(&format!("# BIM/CIM IFC: {proj}\n\n"));
    } else {
        out.push_str("# BIM/CIM IFC モデル概要\n\n");
    }

    out.push_str("## メタデータ\n\n");
    out.push_str("| 項目 | 内容 |\n| --- | --- |\n");
    if !summary.schema.is_empty() {
        out.push_str(&format!("| スキーマ | {} |\n", summary.schema));
    }
    if !summary.description.is_empty() {
        out.push_str(&format!("| 説明 | {} |\n", summary.description));
    }
    if !summary.file_name.is_empty() {
        out.push_str(&format!("| ファイル名 | {} |\n", summary.file_name));
    }
    if !summary.timestamp.is_empty() {
        out.push_str(&format!("| 作成日時 | {} |\n", summary.timestamp));
    }
    if !summary.authors.is_empty() {
        out.push_str(&format!("| 作成者 | {} |\n", summary.authors.join(", ")));
    }
    if !summary.organizations.is_empty() {
        out.push_str(&format!("| 組織 | {} |\n", summary.organizations.join(", ")));
    }
    if !summary.originating_system.is_empty() {
        out.push_str(&format!("| 生成システム | {} |\n", summary.originating_system));
    }
    out.push_str(&format!("| 総エンティティ数 | {} |\n\n", summary.total_entities));

    if !summary.projects.is_empty()
        || !summary.sites.is_empty()
        || !summary.buildings.is_empty()
        || !summary.storeys.is_empty()
    {
        out.push_str("## 空間階層\n\n");
        if !summary.projects.is_empty() {
            out.push_str(&format!("- **Project**: {}\n", summary.projects.join(", ")));
        }
        if !summary.sites.is_empty() {
            out.push_str(&format!("- **Site**: {}\n", summary.sites.join(", ")));
        }
        if !summary.buildings.is_empty() {
            out.push_str(&format!("- **Building**: {}\n", summary.buildings.join(", ")));
        }
        if !summary.storeys.is_empty() {
            out.push_str("- **Storeys**:\n");
            for (name, elev) in &summary.storeys {
                match elev {
                    Some(e) => out.push_str(&format!("  - {name} (標高: {e:.3})\n")),
                    None => out.push_str(&format!("  - {name}\n")),
                }
            }
        }
        out.push('\n');
    }

    if !summary.entity_counts.is_empty() {
        out.push_str("## 主要エンティティ内訳\n\n");
        out.push_str("| エンティティ種別 | 件数 |\n| --- | ---: |\n");
        for (kind, count) in summary.entity_counts.iter().take(30) {
            out.push_str(&format!("| `{kind}` | {count} |\n"));
        }
    }

    Ok(out)
}

fn split_step_statements(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\'' {
            current.push(ch);
            if in_string && chars.peek() == Some(&'\'') {
                current.push(chars.next().unwrap());
            } else {
                in_string = !in_string;
            }
        } else if ch == ';' && !in_string {
            out.push(std::mem::take(&mut current));
        } else {
            current.push(ch);
        }
    }
    if !current.trim().is_empty() {
        out.push(current);
    }
    out
}

fn extract_call_args<'a>(stmt: &'a str, name: &str) -> Option<&'a str> {
    let trimmed = stmt.trim();
    let upper = trimmed.to_ascii_uppercase();
    if !upper.starts_with(name) {
        return None;
    }
    let rest = trimmed[name.len()..].trim_start();
    if rest.starts_with('(') && rest.ends_with(')') {
        Some(&rest[1..rest.len() - 1])
    } else {
        None
    }
}

fn parse_entity_instance(stmt: &str) -> Option<(String, &str)> {
    let trimmed = stmt.trim();
    if !trimmed.starts_with('#') {
        return None;
    }
    let eq_pos = trimmed.find('=')?;
    let rhs = trimmed[eq_pos + 1..].trim();
    let open_paren = rhs.find('(')?;
    if !rhs.ends_with(')') {
        return None;
    }
    let entity_name = rhs[..open_paren].trim().to_ascii_uppercase();
    if entity_name.is_empty() {
        return None;
    }
    let args = &rhs[open_paren + 1..rhs.len() - 1];
    Some((entity_name, args))
}

fn split_top_level_args(args: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut start = 0usize;
    let bytes = args.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\'' {
            if in_string && i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                i += 2;
                continue;
            }
            in_string = !in_string;
        } else if !in_string {
            if b == b'(' {
                depth += 1;
            } else if b == b')' {
                depth = depth.saturating_sub(1);
            } else if b == b',' && depth == 0 {
                out.push(args[start..i].trim());
                start = i + 1;
            }
        }
        i += 1;
    }
    if start <= args.len() {
        let tail = args[start..].trim();
        if !tail.is_empty() || !out.is_empty() {
            out.push(tail);
        }
    }
    out
}

fn extract_string_list(arg: &str) -> Vec<String> {
    let trimmed = arg.trim();
    if trimmed == "$" || trimmed == "*" {
        return Vec::new();
    }
    let inner = if trimmed.starts_with('(') && trimmed.ends_with(')') {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };
    split_top_level_args(inner)
        .into_iter()
        .filter_map(parse_step_string)
        .filter(|s| !s.is_empty())
        .collect()
}

fn parse_step_string(arg: &str) -> Option<String> {
    let trimmed = arg.trim();
    if trimmed == "$" || trimmed == "*" || !trimmed.starts_with('\'') || !trimmed.ends_with('\'') || trimmed.len() < 2 {
        return None;
    }
    let inner = &trimmed[1..trimmed.len() - 1];
    Some(decode_step_string(&inner.replace("''", "'")))
}

fn decode_step_string(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(pos) = rest.find("\\X2\\") {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 4..];
        if let Some(end) = after.find("\\X0\\") {
            let hex = &after[..end];
            let mut units = Vec::new();
            let bytes = hex.as_bytes();
            let mut i = 0;
            while i + 3 < bytes.len() {
                if let Ok(u) = u16::from_str_radix(&hex[i..i + 4], 16) {
                    units.push(u);
                }
                i += 4;
            }
            if !units.is_empty() {
                out.push_str(&String::from_utf16_lossy(&units));
            }
            rest = &after[end + 4..];
        } else {
            out.push_str("\\X2\\");
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_IFC: &str = r#"ISO-10303-21;
HEADER;
FILE_DESCRIPTION(('ViewDefinition [CoordinationView_V2.0]'),'2;1');
FILE_NAME('bridge_model.ifc','2026-04-01T10:00:00',('Taro'),('MLIT'),'IfcOpenShell','BIM-Tool','');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#1=IFCPROJECT('0001',$,'\X2\5EFA7269\X0\A',$,$,$,$,$,$);
#2=IFCSITE('0002',$,'Tokyo Site',$,$,$,$,$,$,$,$,$,$,$);
#3=IFCBUILDING('0003',$,'Main Building',$,$,$,$,$,$,$,$,$);
#4=IFCBUILDINGSTOREY('0004',$,'1F',$,$,$,$,$,$,0.0);
#5=IFCBUILDINGSTOREY('0005',$,'2F',$,$,$,$,$,$,3.5);
#6=IFCWALL('0006',$,'Wall-1',$,$,$,$,$);
#7=IFCWALL('0007',$,'Wall-2',$,$,$,$,$);
ENDSEC;
END-ISO-10303-21;
"#;

    #[test]
    fn parses_ifc_header_hierarchy_and_renders_markdown() {
        assert!(is_ifc(SAMPLE_IFC.as_bytes()));
        assert!(!is_ifc(b"ISO-10303-21;\nHEADER;\nFILE_SCHEMA(('SCADEC'));\nENDSEC;"));

        let summary = inspect_ifc(SAMPLE_IFC.as_bytes()).expect("inspect_ifc");
        assert_eq!(summary.schema, "IFC4");
        assert_eq!(summary.file_name, "bridge_model.ifc");
        assert_eq!(summary.projects, vec!["建物A"]);
        assert_eq!(summary.sites, vec!["Tokyo Site"]);
        assert_eq!(summary.buildings, vec!["Main Building"]);
        assert_eq!(summary.storeys.len(), 2);
        assert_eq!(summary.storeys[1], ("2F".to_string(), Some(3.5)));
        assert_eq!(summary.total_entities, 7);

        let md = to_markdown(SAMPLE_IFC.as_bytes()).expect("to_markdown");
        assert!(md.contains("BIM/CIM IFC: 建物A"));
        assert!(md.contains("IFC4"));
        assert!(md.contains("2F (標高: 3.500)"));
        assert!(md.contains("`IFCWALL` | 2"));
    }
}
