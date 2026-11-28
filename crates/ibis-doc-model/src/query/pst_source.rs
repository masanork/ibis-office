// SPDX-License-Identifier: AGPL-3.0-only
use super::{QColumn, QTable, QType};
use crate::CellValue;
use outlook_pst::{
    ltp::prop_context::PropertyValue,
    messaging::folder::Folder,
    messaging::store::{EntryId, Store},
    ndb::node_id::NodeId,
    open_store,
};
use serde_json::json;
use std::{path::Path, rc::Rc};

const MAX_FOLDERS: usize = 4_096;
const MAX_ROWS: usize = 1_048_575;
const MAX_CELLS: usize = 5_000_000;
const MAX_TEXT_BYTES: usize = 128 * 1024 * 1024;

pub fn is_pst_extension(path: &Path) -> bool {
    path.extension().is_some_and(|ext| {
        ["ost", "pst"]
            .iter()
            .any(|name| ext.eq_ignore_ascii_case(name))
    })
}

fn open_pst_store(path: &Path) -> Result<Rc<dyn Store>, String> {
    if !path.is_file() {
        return Err("Select a local Outlook (.ost/.pst) file".into());
    }
    open_store(path).map_err(|error| format!("Failed to open Outlook store: {error}"))
}

fn prop_string(val: &PropertyValue) -> Option<String> {
    match val {
        PropertyValue::Unicode(v) => Some(v.to_string()),
        PropertyValue::String8(v) => Some(v.to_string()),
        _ => None,
    }
}

fn filetime_to_iso(ft: i64) -> Option<String> {
    // Windows FILETIME: 100-ns intervals since 1601-01-01.
    // Unix epoch (1970-01-01) is 11644473600 seconds after 1601-01-01.
    const EPOCH_DIFF_SECS: i64 = 11_644_473_600;
    if ft <= 0 {
        return None;
    }
    let total_secs = ft / 10_000_000;
    let unix_secs = total_secs.checked_sub(EPOCH_DIFF_SECS)?;
    let days = unix_secs.div_euclid(86_400);
    let secs_of_day = unix_secs.rem_euclid(86_400);
    let hour = secs_of_day / 3_600;
    let minute = (secs_of_day % 3_600) / 60;
    let second = secs_of_day % 60;

    // Howard Hinnant's civil_from_days algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + i64::from(m <= 2);

    Some(format!(
        "{year:04}-{m:02}-{d:02} {hour:02}:{minute:02}:{second:02}"
    ))
}

struct FolderInfo {
    entry_id: EntryId,
    folder_id: u32,
    name: String,
    path: String,
    content_count: i32,
    unread_count: i32,
    folder: Rc<dyn Folder>,
}

fn collect_folders(store: &Rc<dyn Store>) -> Result<Vec<FolderInfo>, String> {
    let mut out = Vec::new();
    let root_table = store.root_hierarchy_table().map_err(|e| e.to_string())?;
    let store_props = store.properties();
    let mut queue: Vec<(EntryId, u32, String)> = Vec::new();

    for row in root_table.rows_matrix() {
        let node_id = NodeId::from(u32::from(row.id()));
        if let Ok(entry_id) = store_props.make_entry_id(node_id) {
            queue.push((entry_id, u32::from(node_id), String::new()));
        }
    }

    while let Some((entry_id, folder_id, parent_path)) = queue.pop() {
        if out.len() >= MAX_FOLDERS {
            return Err("Outlook store has too many folders to list".into());
        }
        let Ok(folder) = store.open_folder(&entry_id) else {
            continue;
        };
        let props = folder.properties();
        let name = props
            .display_name()
            .unwrap_or_else(|_| format!("Folder-{folder_id}"));
        let path = if parent_path.is_empty() {
            format!("/{name}")
        } else {
            format!("{parent_path}/{name}")
        };
        let content_count = props.content_count().unwrap_or(0);
        let unread_count = props.unread_count().unwrap_or(0);

        if let Some(sub_table) = folder.hierarchy_table() {
            let mut sub_entries = Vec::new();
            for row in sub_table.rows_matrix() {
                let node_id = NodeId::from(u32::from(row.id()));
                if let Ok(sub_id) = store_props.make_entry_id(node_id) {
                    sub_entries.push((sub_id, u32::from(node_id), path.clone()));
                }
            }
            for item in sub_entries.into_iter().rev() {
                queue.push(item);
            }
        }

        out.push(FolderInfo {
            entry_id,
            folder_id,
            name,
            path,
            content_count,
            unread_count,
            folder,
        });
    }
    Ok(out)
}

/// List folder hierarchy and item counts in an Outlook (.ost / .pst) file.
pub fn pst_list_folders(path: &Path) -> Result<serde_json::Value, String> {
    let store = open_pst_store(path)?;
    let display_name = store.properties().display_name().unwrap_or_default();
    let folders = collect_folders(&store)?;
    let items: Vec<_> = folders
        .into_iter()
        .map(|f| {
            let _ = f.entry_id;
            json!({
                "folder_id": f.folder_id,
                "name": f.name,
                "path": f.path,
                "content_count": f.content_count,
                "unread_count": f.unread_count,
            })
        })
        .collect();
    Ok(json!({
        "format": "pst_ost",
        "display_name": display_name,
        "folders": items,
    }))
}

fn table_schema(table: &str) -> Result<Vec<QColumn>, String> {
    match table {
        "folders" => Ok(vec![
            QColumn { name: "folder_id".into(), ty: QType::Whole },
            QColumn { name: "name".into(), ty: QType::Text },
            QColumn { name: "path".into(), ty: QType::Text },
            QColumn { name: "content_count".into(), ty: QType::Whole },
            QColumn { name: "unread_count".into(), ty: QType::Whole },
        ]),
        "messages" => Ok(vec![
            QColumn { name: "folder_path".into(), ty: QType::Text },
            QColumn { name: "subject".into(), ty: QType::Text },
            QColumn { name: "sender_name".into(), ty: QType::Text },
            QColumn { name: "sender_email".into(), ty: QType::Text },
            QColumn { name: "display_to".into(), ty: QType::Text },
            QColumn { name: "display_cc".into(), ty: QType::Text },
            QColumn { name: "delivery_time".into(), ty: QType::Text },
            QColumn { name: "size".into(), ty: QType::Whole },
            QColumn { name: "has_attachments".into(), ty: QType::Logical },
            QColumn { name: "body".into(), ty: QType::Text },
        ]),
        _ => Err(format!(
            "Unknown Outlook virtual table '{table}'; expected 'folders' or 'messages'"
        )),
    }
}

fn resolve_projection(
    table: &str,
    selected: &[String],
) -> Result<(Vec<QColumn>, Vec<usize>), String> {
    let all = table_schema(table)?;
    if selected.is_empty() {
        let indices = (0..all.len()).collect();
        return Ok((all, indices));
    }
    let mut columns = Vec::with_capacity(selected.len());
    let mut indices = Vec::with_capacity(selected.len());
    for name in selected {
        let Some((idx, col)) = all.iter().enumerate().find(|(_, c)| c.name == *name) else {
            return Err(format!(
                "Column '{name}' does not exist in Outlook table '{table}'"
            ));
        };
        columns.push(col.clone());
        indices.push(idx);
    }
    Ok((columns, indices))
}

/// Return SQL-visible column definitions for an Outlook (.ost / .pst) virtual table.
pub fn pst_dataset_columns(
    path: &Path,
    table: &str,
    selected: &[String],
) -> Result<Vec<QColumn>, String> {
    let _ = open_pst_store(path)?;
    let (columns, _) = resolve_projection(table, selected)?;
    Ok(columns)
}

/// Stream one Outlook virtual table ('folders' or 'messages') into a visitor.
pub(super) fn visit_pst_table(
    path: &Path,
    table: &str,
    selected: &[String],
    schema_only: bool,
    mut visit: impl FnMut(&[QColumn], Option<Vec<CellValue>>) -> Result<(), String>,
) -> Result<(), String> {
    let store = open_pst_store(path)?;
    let (columns, indices) = resolve_projection(table, selected)?;
    visit(&columns, None)?;
    if schema_only {
        return Ok(());
    }

    let folders = collect_folders(&store)?;
    let mut count = 0usize;

    match table {
        "folders" => {
            for f in folders {
                count += 1;
                if count > MAX_ROWS || columns.len().saturating_mul(count) > MAX_CELLS {
                    return Err("Outlook folders table exceeds the row or cell limit".into());
                }
                let full = [
                    CellValue::Number(f64::from(f.folder_id)),
                    CellValue::Text(f.name),
                    CellValue::Text(f.path),
                    CellValue::Number(f64::from(f.content_count)),
                    CellValue::Number(f64::from(f.unread_count)),
                ];
                let row = indices.iter().map(|&i| full[i].clone()).collect();
                visit(&columns, Some(row))?;
            }
        }
        "messages" => {
            let store_props = store.properties();
            for f in folders {
                let Some(contents) = f.folder.contents_table() else {
                    continue;
                };
                for row in contents.rows_matrix() {
                    let node_id = NodeId::from(u32::from(row.id()));
                    let Ok(entry_id) = store_props.make_entry_id(node_id) else {
                        continue;
                    };
                    let Ok(msg) = store.open_message(&entry_id, None) else {
                        continue;
                    };
                    let props = msg.properties();
                    let get_str = |tag: u16| -> CellValue {
                        props
                            .get(tag)
                            .and_then(prop_string)
                            .map(CellValue::Text)
                            .unwrap_or(CellValue::Empty)
                    };
                    let delivery_time = props
                        .get(0x0E06)
                        .and_then(|v| match v {
                            PropertyValue::Time(t) => filetime_to_iso(*t).map(CellValue::Text),
                            _ => None,
                        })
                        .unwrap_or(CellValue::Empty);
                    let size = props
                        .get(0x0E08)
                        .and_then(|v| match v {
                            PropertyValue::Integer32(n) => Some(CellValue::Number(f64::from(*n))),
                            _ => None,
                        })
                        .unwrap_or(CellValue::Empty);
                    let has_attachments = props
                        .get(0x0E1B)
                        .and_then(|v| match v {
                            PropertyValue::Boolean(b) => Some(CellValue::Bool(*b)),
                            _ => None,
                        })
                        .unwrap_or(CellValue::Empty);

                    count += 1;
                    if count > MAX_ROWS || columns.len().saturating_mul(count) > MAX_CELLS {
                        return Err("Outlook messages table exceeds the row or cell limit".into());
                    }
                    let full = [
                        CellValue::Text(f.path.clone()),
                        get_str(0x0037),
                        get_str(0x0C1A),
                        get_str(0x0C1F),
                        get_str(0x0E04),
                        get_str(0x0E03),
                        delivery_time,
                        size,
                        has_attachments,
                        get_str(0x1000),
                    ];
                    let projected = indices.iter().map(|&i| full[i].clone()).collect();
                    visit(&columns, Some(projected))?;
                }
            }
        }
        _ => unreachable!(),
    }

    Ok(())
}

/// Read one Outlook virtual table into memory within explicit cell and text budgets.
pub fn read_pst_table_bounded(
    path: &Path,
    table: &str,
    selected: &[String],
    max_cells: usize,
    max_text: usize,
) -> Result<QTable, String> {
    let mut table_out = QTable::default();
    let mut text_budget = max_text.min(MAX_TEXT_BYTES);
    let mut cell_budget = max_cells.min(MAX_CELLS);
    visit_pst_table(path, table, selected, false, |columns, row| {
        if let Some(row) = row {
            cell_budget = cell_budget
                .checked_sub(columns.len())
                .ok_or("Outlook table exceeds the in-memory cell limit")?;
            super::sqlite_source::charge_dataset_text(&row, &mut text_budget)?;
            table_out.rows.push(row);
        } else {
            table_out.columns = columns.to_vec();
        }
        Ok(())
    })?;
    Ok(table_out)
}

/// Materialize Outlook (.ost / .pst) 'folders' and 'messages' virtual tables into a standalone SQLite file via Turso.
pub fn pst_export_sqlite(
    input_path: &Path,
    output_path: &Path,
    tables_filter: Option<&[String]>,
    overwrite: bool,
) -> Result<serde_json::Value, String> {
    let _ = open_pst_store(input_path)?;
    let target_tables: Vec<String> = match tables_filter {
        Some(filter) => {
            if filter.is_empty() {
                return Err("tables filter must not be empty".into());
            }
            for name in filter {
                if !matches!(name.as_str(), "folders" | "messages") {
                    return Err(format!(
                        "Unknown Outlook virtual table '{name}'; expected 'folders' or 'messages'"
                    ));
                }
            }
            filter.to_vec()
        }
        None => vec!["folders".into(), "messages".into()],
    };
    let sources: Vec<_> = target_tables
        .into_iter()
        .map(|name| {
            (
                name.clone(),
                super::sqlite_source::DiskDataset::Pst(input_path.to_path_buf(), name, Vec::new()),
            )
        })
        .collect();
    super::sqlite_source::export_datasets_to_sqlite(&sources, output_path, overwrite)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_virtual_table_schemas_and_rejects_invalid_files() {
        let (cols, idx) = resolve_projection("messages", &["subject".into(), "body".into()]).unwrap();
        assert_eq!(cols.len(), 2);
        assert_eq!(idx, vec![1, 9]);
        assert!(resolve_projection("unknown", &[]).is_err());
        assert!(filetime_to_iso(133_500_000_000_000_000).is_some());

        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.ost");
        std::fs::write(&bad, b"not an ost file").unwrap();
        assert!(pst_list_folders(&bad).is_err());
    }
}
