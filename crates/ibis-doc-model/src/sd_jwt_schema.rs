//! Custom SD-JWT schema definition, JSON import/export, worksheet inference, and scaffolding.

use std::collections::BTreeMap;

use crate::command::Command;
use crate::model::{Presentation, SlideId};
use crate::sd_jwt_binding::{
    BindingSource, ClaimValue, abs_cell_ref, clear_sd_jwt_bindings_commands,
    extract_sheet_claims, slide_sheet_formula_prefix,
};
use crate::sheet::{CellStyle, CellValue};
use crate::sheet_calc::sheet_of_slide;
use crate::sheet_command::{CellInput, PutCell, SheetCommand};

/// Supported claim value types in an `SdJwtSchema`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SdJwtSchemaValueType {
    String,
    Number,
    Boolean,
    Date,
    StringArray,
    ObjectTable,
}

impl SdJwtSchemaValueType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Number => "number",
            Self::Boolean => "boolean",
            Self::Date => "date",
            Self::StringArray => "string_array",
            Self::ObjectTable => "object_table",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "number" | "integer" | "int" | "float" => Self::Number,
            "boolean" | "bool" => Self::Boolean,
            "date" | "full-date" => Self::Date,
            "string_array" | "array" | "string[]" => Self::StringArray,
            "object_table" | "table" | "object[]" => Self::ObjectTable,
            _ => Self::String,
        }
    }
}

/// Column definition for a 2D `ObjectTable` claim (e.g. `driving_privileges`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SdJwtSchemaTableColumn {
    pub key: String,
    pub label: Option<String>,
    pub value_type: SdJwtSchemaValueType,
}

/// Single claim definition inside an `SdJwtSchema`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SdJwtSchemaClaim {
    pub path: String,
    pub label: Option<String>,
    pub value_type: SdJwtSchemaValueType,
    pub selective: bool,
    pub disclose_default: bool,
    pub sample_value: Option<String>,
    pub columns: Vec<SdJwtSchemaTableColumn>,
}

/// Portable SD-JWT credential schema (compatible with SD-JWT VC Type Metadata concepts).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SdJwtSchema {
    pub vct: String,
    pub name: String,
    pub description: Option<String>,
    pub claims: Vec<SdJwtSchemaClaim>,
}

impl SdJwtSchema {
    /// Converts this schema into a deterministic `ClaimValue::Object`.
    pub fn to_claim_value(&self) -> ClaimValue {
        let mut root = BTreeMap::new();
        root.insert("vct".into(), ClaimValue::String(self.vct.clone()));
        root.insert("name".into(), ClaimValue::String(self.name.clone()));
        if let Some(desc) = &self.description
            && !desc.trim().is_empty()
        {
            root.insert("description".into(), ClaimValue::String(desc.clone()));
        }
        let claims_arr = self
            .claims
            .iter()
            .map(|c| {
                let mut obj = BTreeMap::new();
                obj.insert("path".into(), ClaimValue::String(c.path.clone()));
                if let Some(lbl) = &c.label
                    && !lbl.trim().is_empty()
                {
                    obj.insert("label".into(), ClaimValue::String(lbl.clone()));
                }
                obj.insert(
                    "type".into(),
                    ClaimValue::String(c.value_type.as_str().to_string()),
                );
                obj.insert("sd".into(), ClaimValue::Bool(c.selective));
                if c.selective {
                    obj.insert(
                        "disclose_default".into(),
                        ClaimValue::Bool(c.disclose_default),
                    );
                }
                if let Some(sample) = &c.sample_value
                    && !sample.trim().is_empty()
                {
                    obj.insert("sample".into(), ClaimValue::String(sample.clone()));
                }
                if !c.columns.is_empty() {
                    let cols = c
                        .columns
                        .iter()
                        .map(|col| {
                            let mut cobj = BTreeMap::new();
                            cobj.insert("key".into(), ClaimValue::String(col.key.clone()));
                            if let Some(lbl) = &col.label {
                                cobj.insert("label".into(), ClaimValue::String(lbl.clone()));
                            }
                            cobj.insert(
                                "type".into(),
                                ClaimValue::String(col.value_type.as_str().to_string()),
                            );
                            ClaimValue::Object(cobj)
                        })
                        .collect();
                    obj.insert("columns".into(), ClaimValue::Array(cols));
                }
                ClaimValue::Object(obj)
            })
            .collect();
        root.insert("claims".into(), ClaimValue::Array(claims_arr));
        ClaimValue::Object(root)
    }

    /// Serializes this schema as pretty-printed JSON.
    pub fn to_json_pretty(&self) -> String {
        self.to_claim_value().to_json_pretty()
    }

    /// Parses an `SdJwtSchema` from JSON.
    /// Accepts both `ibis-office` schema JSON and SD-JWT VC Type Metadata (`claims[].path` arrays, `sd: "always"|"allowed"|"never"`, `display[].label`).
    pub fn from_json(json: &str) -> Result<Self, String> {
        let root = ClaimValue::parse_json(json)?;
        let obj = root
            .as_object()
            .ok_or_else(|| "schema root must be a JSON object".to_string())?;

        let vct = obj
            .get("vct")
            .and_then(ClaimValue::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("urn:example:custom:1")
            .to_string();
        let name = obj
            .get("name")
            .and_then(ClaimValue::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Custom Credential")
            .to_string();
        let description = obj
            .get("description")
            .and_then(ClaimValue::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let claims_val = obj
            .get("claims")
            .and_then(ClaimValue::as_array)
            .ok_or_else(|| "schema must contain a 'claims' array".to_string())?;

        let mut claims = Vec::new();
        for item in claims_val {
            let Some(cobj) = item.as_object() else {
                continue;
            };
            let path = match cobj.get("path") {
                Some(ClaimValue::String(s)) => s.trim().to_string(),
                Some(ClaimValue::Array(parts)) => parts
                    .iter()
                    .filter_map(ClaimValue::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join("."),
                _ => String::new(),
            };
            if path.is_empty() {
                continue;
            }
            let label = cobj
                .get("label")
                .and_then(ClaimValue::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    cobj.get("display")
                        .and_then(ClaimValue::as_array)
                        .and_then(|arr| arr.first())
                        .and_then(ClaimValue::as_object)
                        .and_then(|d| d.get("label"))
                        .and_then(ClaimValue::as_str)
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                });
            let value_type = cobj
                .get("type")
                .and_then(ClaimValue::as_str)
                .map(SdJwtSchemaValueType::parse)
                .unwrap_or(SdJwtSchemaValueType::String);

            let selective = match cobj.get("sd") {
                Some(ClaimValue::Bool(b)) => *b,
                Some(ClaimValue::String(s)) => {
                    let lower = s.trim().to_ascii_lowercase();
                    lower != "never" && lower != "false" && lower != "always_disclosed"
                }
                _ => true,
            };
            let disclose_default = cobj
                .get("disclose_default")
                .and_then(ClaimValue::as_bool)
                .unwrap_or(selective);
            let sample_value = cobj
                .get("sample")
                .and_then(|v| match v {
                    ClaimValue::String(s) => Some(s.clone()),
                    ClaimValue::Int(n) => Some(n.to_string()),
                    ClaimValue::Float(f) => Some(f.to_string()),
                    ClaimValue::Bool(b) => Some(b.to_string()),
                    _ => None,
                })
                .filter(|s| !s.is_empty());

            let columns = cobj
                .get("columns")
                .and_then(ClaimValue::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|col| match col {
                            ClaimValue::String(k) if !k.trim().is_empty() => {
                                Some(SdJwtSchemaTableColumn {
                                    key: k.trim().to_string(),
                                    label: None,
                                    value_type: SdJwtSchemaValueType::String,
                                })
                            }
                            ClaimValue::Object(m) => {
                                let key = m
                                    .get("key")
                                    .and_then(ClaimValue::as_str)
                                    .map(str::trim)
                                    .filter(|s| !s.is_empty())?
                                    .to_string();
                                let label = m
                                    .get("label")
                                    .and_then(ClaimValue::as_str)
                                    .map(|s| s.trim().to_string())
                                    .filter(|s| !s.is_empty());
                                let value_type = m
                                    .get("type")
                                    .and_then(ClaimValue::as_str)
                                    .map(SdJwtSchemaValueType::parse)
                                    .unwrap_or(SdJwtSchemaValueType::String);
                                Some(SdJwtSchemaTableColumn {
                                    key,
                                    label,
                                    value_type,
                                })
                            }
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default();

            claims.push(SdJwtSchemaClaim {
                path,
                label,
                value_type,
                selective,
                disclose_default,
                sample_value,
                columns,
            });
        }

        if claims.is_empty() {
            return Err("schema must contain at least one valid claim".to_string());
        }

        Ok(Self {
            vct,
            name,
            description,
            claims,
        })
    }

    /// Infers an `SdJwtSchema` from the `claim.*` / `sd.*` / `disclose.*` bindings on `slide_id`.
    pub fn infer_from_sheet(
        presentation: &Presentation,
        slide_id: SlideId,
        name: Option<&str>,
    ) -> Option<Self> {
        let bindings = extract_sheet_claims(presentation, slide_id)?;
        if bindings.claims.is_empty() {
            return None;
        }
        let sheet = presentation.slide(slide_id).and_then(sheet_of_slide);

        let vct = bindings
            .claims
            .iter()
            .find(|c| c.path_str == "vct")
            .and_then(|c| c.value.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("urn:example:custom:1")
            .to_string();

        let schema_name = name
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                presentation
                    .workbook
                    .as_ref()
                    .and_then(|wb| wb.sheets.iter().find(|ws| ws.slide == Some(slide_id)))
                    .map(|ws| ws.name.clone())
            })
            .unwrap_or_else(|| "Custom Credential".to_string());

        let mut claims = Vec::new();
        for bc in &bindings.claims {
            if bc.path_str == "vct" || bc.path_str == "iss" {
                continue;
            }
            let (value_type, sample_value, columns) = infer_claim_shape(&bc.value);
            let disclose_default = if bc.selective {
                bindings
                    .disclosures
                    .get(&bc.path_str)
                    .map(|d| d.enabled)
                    .unwrap_or(true)
            } else {
                false
            };

            let label = match bc.source {
                BindingSource::Cell { row, column } => sheet.and_then(|sh| {
                    if column > 0 {
                        sh.cells
                            .iter()
                            .find(|c| c.row == row && c.column == column - 1)
                            .map(|c| c.text.trim().to_string())
                            .filter(|s| !s.is_empty())
                    } else {
                        None
                    }
                }),
                BindingSource::Control { .. } => None,
            };

            claims.push(SdJwtSchemaClaim {
                path: bc.path_str.clone(),
                label,
                value_type,
                selective: bc.selective,
                disclose_default,
                sample_value,
                columns,
            });
        }

        if claims.is_empty() {
            return None;
        }

        Some(Self {
            vct,
            name: schema_name,
            description: None,
            claims,
        })
    }

    /// Generates undoable worksheet commands to scaffold a credential form from this schema.
    pub fn build_sheet_commands(&self, presentation: &Presentation, slide_id: SlideId) -> Vec<Command> {
        let date1904 = presentation.workbook.as_ref().is_some_and(|w| w.date1904);
        let sheet_prefix = slide_sheet_formula_prefix(presentation, slide_id);

        let mut header_style = CellStyle::default();
        header_style.bold = true;

        let mut date_style = CellStyle::default();
        date_style.number_format = "yyyy-mm-dd".to_string();

        let mut text_style = CellStyle::default();
        text_style.number_format = "@".to_string();

        let mut checkbox_style = CellStyle::default();
        checkbox_style.checkbox = true;

        let mut cells: Vec<PutCell> = Vec::new();
        let mut name_cmds = clear_sd_jwt_bindings_commands(presentation, slide_id);

        let put_text = |cells: &mut Vec<PutCell>, r: u32, c: u32, s: &str, style: Option<CellStyle>| {
            cells.push(PutCell {
                row: r,
                column: c,
                input: CellInput::Value(CellValue::Text(s.to_string())),
                style,
            });
        };

        put_text(
            &mut cells,
            0,
            0,
            &format!("{} (Claim)", self.name),
            Some(header_style.clone()),
        );
        put_text(&mut cells, 0, 1, "Value", Some(header_style.clone()));
        put_text(&mut cells, 0, 2, "Disclose", Some(header_style.clone()));

        // Row 1: claim.vct
        put_text(&mut cells, 1, 0, "vct", None);
        put_text(&mut cells, 1, 1, &self.vct, None);
        put_text(&mut cells, 1, 2, "Always (claim.vct)", None);
        name_cmds.push(Command::Sheet(SheetCommand::SetName {
            name: "claim.vct".into(),
            sheet: None,
            formula: Some(format!("{sheet_prefix}{}", abs_cell_ref(1, 1))),
        }));

        let mut row: u32 = 2;
        for claim in &self.claims {
            let label = claim.label.as_deref().unwrap_or(&claim.path);
            match claim.value_type {
                SdJwtSchemaValueType::ObjectTable => {
                    let cols = if claim.columns.is_empty() {
                        vec![
                            SdJwtSchemaTableColumn {
                                key: "code".into(),
                                label: None,
                                value_type: SdJwtSchemaValueType::String,
                            },
                            SdJwtSchemaTableColumn {
                                key: "value".into(),
                                label: None,
                                value_type: SdJwtSchemaValueType::String,
                            },
                        ]
                    } else {
                        claim.columns.clone()
                    };
                    put_text(&mut cells, row, 0, label, Some(header_style.clone()));
                    if claim.selective {
                        cells.push(PutCell {
                            row,
                            column: 1,
                            input: CellInput::Value(CellValue::Bool(claim.disclose_default)),
                            style: Some(checkbox_style.clone()),
                        });
                        name_cmds.push(Command::Sheet(SheetCommand::SetName {
                            name: format!("disclose.{}", claim.path),
                            sheet: None,
                            formula: Some(format!("{sheet_prefix}{}", abs_cell_ref(1, row))),
                        }));
                    } else {
                        put_text(&mut cells, row, 1, "Always", None);
                    }
                    let table_header_row = row + 1;
                    let sample_row = row + 2;
                    for (ci, col) in cols.iter().enumerate() {
                        let c = ci as u32;
                        put_text(&mut cells, table_header_row, c, &col.key, Some(header_style.clone()));
                        let sample = match col.value_type {
                            SdJwtSchemaValueType::Boolean => {
                                CellInput::Value(CellValue::Bool(true))
                            }
                            SdJwtSchemaValueType::Number => CellInput::Value(CellValue::Number(1.0)),
                            _ => CellInput::Value(CellValue::Text("sample".into())),
                        };
                        cells.push(PutCell {
                            row: sample_row,
                            column: c,
                            input: sample,
                            style: None,
                        });
                    }
                    let last_col = (cols.len().saturating_sub(1)) as u32;
                    let prefix = if claim.selective { "sd" } else { "claim" };
                    name_cmds.push(Command::Sheet(SheetCommand::SetName {
                        name: format!("{prefix}.{}", claim.path),
                        sheet: None,
                        formula: Some(format!(
                            "{sheet_prefix}{}:{}",
                            abs_cell_ref(0, table_header_row),
                            abs_cell_ref(last_col, sample_row)
                        )),
                    }));
                    row = sample_row + 1;
                }
                SdJwtSchemaValueType::StringArray => {
                    let items: Vec<&str> = claim
                        .sample_value
                        .as_deref()
                        .map(|s| {
                            s.split(',')
                                .map(str::trim)
                                .filter(|x| !x.is_empty())
                                .collect()
                        })
                        .filter(|v: &Vec<&str>| !v.is_empty())
                        .unwrap_or_else(|| vec!["item1", "item2"]);
                    put_text(&mut cells, row, 0, label, None);
                    for (i, item) in items.iter().enumerate() {
                        put_text(
                            &mut cells,
                            row,
                            1 + i as u32,
                            item,
                            Some(text_style.clone()),
                        );
                    }
                    let last_val_col = 1 + (items.len().saturating_sub(1)) as u32;
                    let prefix = if claim.selective { "sd" } else { "claim" };
                    name_cmds.push(Command::Sheet(SheetCommand::SetName {
                        name: format!("{prefix}.{}", claim.path),
                        sheet: None,
                        formula: Some(format!(
                            "{sheet_prefix}{}:{}",
                            abs_cell_ref(1, row),
                            abs_cell_ref(last_val_col, row)
                        )),
                    }));
                    if claim.selective {
                        let chk_col = last_val_col + 1;
                        cells.push(PutCell {
                            row,
                            column: chk_col,
                            input: CellInput::Value(CellValue::Bool(claim.disclose_default)),
                            style: Some(checkbox_style.clone()),
                        });
                        name_cmds.push(Command::Sheet(SheetCommand::SetName {
                            name: format!("disclose.{}", claim.path),
                            sheet: None,
                            formula: Some(format!("{sheet_prefix}{}", abs_cell_ref(chk_col, row))),
                        }));
                    }
                    row += 1;
                }
                _ => {
                    put_text(&mut cells, row, 0, label, None);
                    let (val_input, val_style) =
                        sample_cell_input(claim.value_type, claim.sample_value.as_deref(), date1904, &date_style, &text_style);
                    cells.push(PutCell {
                        row,
                        column: 1,
                        input: val_input,
                        style: val_style,
                    });
                    let prefix = if claim.selective { "sd" } else { "claim" };
                    name_cmds.push(Command::Sheet(SheetCommand::SetName {
                        name: format!("{prefix}.{}", claim.path),
                        sheet: None,
                        formula: Some(format!("{sheet_prefix}{}", abs_cell_ref(1, row))),
                    }));
                    if claim.selective {
                        cells.push(PutCell {
                            row,
                            column: 2,
                            input: CellInput::Value(CellValue::Bool(claim.disclose_default)),
                            style: Some(checkbox_style.clone()),
                        });
                        name_cmds.push(Command::Sheet(SheetCommand::SetName {
                            name: format!("disclose.{}", claim.path),
                            sheet: None,
                            formula: Some(format!("{sheet_prefix}{}", abs_cell_ref(2, row))),
                        }));
                    } else {
                        put_text(&mut cells, row, 2, &format!("Always (claim.{})", claim.path), None);
                    }
                    row += 1;
                }
            }
        }

        let mut cmds = vec![Command::Sheet(SheetCommand::PasteCells {
            sheet: slide_id,
            cells,
        })];
        cmds.extend(name_cmds);
        cmds
    }
}

fn sample_cell_input(
    value_type: SdJwtSchemaValueType,
    sample: Option<&str>,
    date1904: bool,
    date_style: &CellStyle,
    text_style: &CellStyle,
) -> (CellInput, Option<CellStyle>) {
    match value_type {
        SdJwtSchemaValueType::Number => {
            let n = sample.and_then(|s| s.trim().parse::<f64>().ok()).unwrap_or(0.0);
            (CellInput::Value(CellValue::Number(n)), None)
        }
        SdJwtSchemaValueType::Boolean => {
            let b = sample
                .map(|s| matches!(s.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes"))
                .unwrap_or(false);
            (CellInput::Value(CellValue::Bool(b)), None)
        }
        SdJwtSchemaValueType::Date => {
            let (y, m, d) = sample
                .and_then(parse_iso_ymd)
                .unwrap_or((2026, 1, 1));
            let serial = ibis_calc::eval::convert::serial_from_date(y, m, d, date1904) as f64;
            (
                CellInput::Value(CellValue::Number(serial)),
                Some(date_style.clone()),
            )
        }
        SdJwtSchemaValueType::String | SdJwtSchemaValueType::StringArray | SdJwtSchemaValueType::ObjectTable => {
            let txt = sample.unwrap_or("sample");
            (
                CellInput::Value(CellValue::Text(txt.to_string())),
                Some(text_style.clone()),
            )
        }
    }
}

fn parse_iso_ymd(raw: &str) -> Option<(i64, i64, i64)> {
    let mut parts = raw.trim().split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    Some((y, m, d))
}

fn infer_claim_shape(
    value: &ClaimValue,
) -> (
    SdJwtSchemaValueType,
    Option<String>,
    Vec<SdJwtSchemaTableColumn>,
) {
    match value {
        ClaimValue::Bool(b) => (SdJwtSchemaValueType::Boolean, Some(b.to_string()), Vec::new()),
        ClaimValue::Int(n) => (SdJwtSchemaValueType::Number, Some(n.to_string()), Vec::new()),
        ClaimValue::Float(f) => (SdJwtSchemaValueType::Number, Some(f.to_string()), Vec::new()),
        ClaimValue::String(s) => {
            let vt = if parse_iso_ymd(s).is_some() && s.len() == 10 {
                SdJwtSchemaValueType::Date
            } else {
                SdJwtSchemaValueType::String
            };
            (vt, Some(s.clone()), Vec::new())
        }
        ClaimValue::Array(items) => {
            if let Some(first_obj) = items.first().and_then(ClaimValue::as_object) {
                let cols = first_obj
                    .iter()
                    .map(|(k, v)| {
                        let (vt, _, _) = infer_claim_shape(v);
                        SdJwtSchemaTableColumn {
                            key: k.clone(),
                            label: None,
                            value_type: vt,
                        }
                    })
                    .collect();
                (SdJwtSchemaValueType::ObjectTable, None, cols)
            } else {
                let joined = items
                    .iter()
                    .filter_map(|v| match v {
                        ClaimValue::String(s) => Some(s.clone()),
                        ClaimValue::Int(n) => Some(n.to_string()),
                        ClaimValue::Float(f) => Some(f.to_string()),
                        ClaimValue::Bool(b) => Some(b.to_string()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                (
                    SdJwtSchemaValueType::StringArray,
                    (!joined.is_empty()).then_some(joined),
                    Vec::new(),
                )
            }
        }
        ClaimValue::Object(_) | ClaimValue::Null => {
            (SdJwtSchemaValueType::String, None, Vec::new())
        }
    }
}
