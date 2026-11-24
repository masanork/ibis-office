// SPDX-License-Identifier: AGPL-3.0-only
use super::{QColumn, QTable, QType};
use crate::CellValue;
use serde::{
    Deserialize, Deserializer,
    de::{DeserializeSeed, MapAccess, Visitor},
};
use serde_json::{Value, value::RawValue};
use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap, HashSet},
    fmt,
    fs::File,
    io::{BufRead, BufReader, Read, Seek},
    path::Path,
};

const MAX_LINE_BYTES: usize = 1024 * 1024;
const MAX_SOURCE_BYTES: usize = 64 * 1024 * 1024;
const MAX_COLUMNS: usize = 16_384;
const MAX_DATA_ROWS: usize = 1_048_575;
const MAX_CELLS: usize = 5_000_000;
const MAX_TEXT_UNITS: usize = 32_767;
const PRECISION_ERROR: &str = "Number exceeds spreadsheet precision";

/// Read one JSON object per line. Absent fields and nulls become empty cells.
pub fn read_json_lines(path: &Path) -> Result<QTable, String> {
    if !path.is_file() {
        return Err("Select a local JSON Lines file".into());
    }
    let file = File::open(path).map_err(|error| error.to_string())?;
    read_records(BufReader::new(file)).map_err(|error| {
        format!(
            "{}: {error}",
            path.file_name().unwrap_or_default().to_string_lossy()
        )
    })
}

fn read_records(reader: impl BufRead) -> Result<QTable, String> {
    let mut output = Records::default();
    visit_lines(reader, MAX_SOURCE_BYTES, |fields, location| {
        output.push(fields, &format!("Line {location}"))
    })?;
    output.finish()
}

fn visit_lines(
    reader: impl BufRead,
    max_bytes: usize,
    mut visit: impl FnMut(BTreeMap<String, &RawValue>, usize) -> Result<(), String>,
) -> Result<(), String> {
    visit_raw_lines(reader, max_bytes, |line, number| {
        let fields = serde_json::from_slice(line).map_err(|error| record_error(number, error))?;
        visit(fields, number)
    })
}

fn record_error(number: usize, error: serde_json::Error) -> String {
    format!("Line {number}: expected one JSON object ({error})")
}

fn visit_raw_lines(
    reader: impl BufRead,
    max_bytes: usize,
    mut visit: impl FnMut(&[u8], usize) -> Result<(), String>,
) -> Result<(), String> {
    scan_lines(reader, max_bytes, false, |line, number| {
        visit(line.expect("strict scan rejects oversized lines"), number)
    })
}

/// Logical byte position after reading a bounded JSON Lines prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JsonLinesSamplePosition {
    pub lines: usize,
    pub bytes: usize,
    pub file_bytes: usize,
}

impl JsonLinesSamplePosition {
    pub fn at_end(self) -> bool {
        self.bytes == self.file_bytes
    }

    /// Round an extrapolation to about two significant digits.
    pub fn estimated_lines(self) -> usize {
        if self.at_end() || self.bytes == 0 {
            return self.lines;
        }
        let estimate = ((self.lines as u128 * self.file_bytes as u128) / self.bytes as u128)
            .min(usize::MAX as u128) as usize;
        let unit = 10usize.pow(estimate.checked_ilog10().unwrap_or(0).saturating_sub(1));
        (estimate.saturating_add(unit / 2) / unit * unit).max(self.lines)
    }
}

fn scan_lines(
    reader: impl BufRead,
    max_bytes: usize,
    recover: bool,
    visit: impl FnMut(Option<&[u8]>, usize) -> Result<(), String>,
) -> Result<(), String> {
    scan_lines_up_to(reader, max_bytes, recover, usize::MAX, visit).map(|_| ())
}

fn scan_lines_up_to(
    reader: impl BufRead,
    max_bytes: usize,
    recover: bool,
    max_lines: usize,
    mut visit: impl FnMut(Option<&[u8]>, usize) -> Result<(), String>,
) -> Result<JsonLinesSamplePosition, String> {
    let mut reader = reader.take((max_bytes as u64).saturating_add(1));
    let mut line = Vec::new();
    let mut bytes = 0usize;
    let mut lines = 0;
    for number in 1..=max_lines {
        line.clear();
        let read = reader
            .by_ref()
            .take((MAX_LINE_BYTES + 1) as u64)
            .read_until(b'\n', &mut line)
            .map_err(|error| format!("Line {number}: {error}"))?;
        if read == 0 {
            break;
        }
        bytes = bytes.saturating_add(read);
        let oversized = read > MAX_LINE_BYTES;
        if oversized && recover && line.last() != Some(&b'\n') {
            let skipped = reader
                .skip_until(b'\n')
                .map_err(|error| format!("Line {number}: {error}"))?;
            bytes = bytes.saturating_add(skipped);
        }
        if bytes > max_bytes || (oversized && !recover) {
            return Err(format!(
                "Line {number}: JSON Lines source exceeds the byte limit"
            ));
        }
        visit((!oversized).then_some(line.as_slice()), number)?;
        lines = number;
    }
    Ok(JsonLinesSamplePosition {
        lines,
        bytes,
        file_bytes: max_bytes,
    })
}

fn merge_type(previous: Option<QType>, next: QType) -> QType {
    match (previous, next) {
        (None, next) => next,
        (Some(previous), next) if previous == next => previous,
        (Some(QType::Whole), QType::Decimal) | (Some(QType::Decimal), QType::Whole) => {
            QType::Decimal
        }
        _ => QType::Any,
    }
}

pub(super) fn visit_json_lines(
    path: &Path,
    selected: &[String],
    schema_only: bool,
    mut visit: impl FnMut(&[QColumn], Option<Vec<CellValue>>) -> Result<(), String>,
) -> Result<(), String> {
    let mut input = BufReader::new(File::open(path).map_err(|error| error.to_string())?);
    let metadata = input
        .get_ref()
        .metadata()
        .map_err(|error| error.to_string())?;
    let max_bytes = usize::try_from(metadata.len()).map_err(|_| "JSON Lines file is too large")?;
    let mut indices = HashMap::new();
    let mut columns = Vec::new();
    let mut types = Vec::new();
    for name in selected {
        add_sql_column(name, &mut indices, &mut columns, &mut types)?;
    }
    // The first pass retains only the schema, including fields first seen late.
    #[cfg(feature = "report-profiling")]
    let schema_timer = super::report_profile::Timer::start(super::report_profile::Phase::Schema);
    if selected.is_empty() {
        visit_lines(&mut input, max_bytes, |fields, line| {
            for (name, raw) in fields {
                let index = match indices.get(&name) {
                    Some(&index) => index,
                    None => add_sql_column(&name, &mut indices, &mut columns, &mut types)?,
                };
                infer_type(&mut types[index], raw, line, &name)?;
            }
            Ok(())
        })?;
    } else {
        visit_projected_lines(&mut input, max_bytes, &indices, |fields, line| {
            for (index, raw) in fields.into_iter().enumerate() {
                if let Some(raw) = raw {
                    infer_type(&mut types[index], raw, line, &columns[index].name)?;
                }
            }
            Ok(())
        })?;
    }
    if columns.is_empty() {
        return Err("JSON source contains no fields".into());
    }
    for (column, ty) in columns.iter_mut().zip(types) {
        column.ty = ty.unwrap_or(QType::Any);
    }
    #[cfg(feature = "report-profiling")]
    drop(schema_timer);
    visit(&columns, None)?;
    if !schema_only {
        input.rewind().map_err(|error| error.to_string())?;
        if selected.is_empty() {
            visit_lines(&mut input, max_bytes, |mut fields, line| {
                let values = columns
                    .iter()
                    .map(|column| sql_cell(fields.remove(&column.name), line, &column.name))
                    .collect::<Result<Vec<_>, _>>()?;
                if !fields.is_empty() {
                    return Err("JSON Lines schema changed during import".into());
                }
                visit(&columns, Some(values))
            })?;
        } else {
            visit_projected_lines(&mut input, max_bytes, &indices, |fields, line| {
                let values = fields
                    .into_iter()
                    .zip(&columns)
                    .map(|(raw, column)| sql_cell(raw, line, &column.name))
                    .collect::<Result<Vec<_>, _>>()?;
                visit(&columns, Some(values))
            })?;
        }
    }
    let after = input
        .get_ref()
        .metadata()
        .map_err(|error| error.to_string())?;
    if metadata.len() != after.len() || metadata.modified().ok() != after.modified().ok() {
        return Err("JSON Lines file changed during import".into());
    }
    Ok(())
}

/// A field observed in valid JSON Lines objects.
#[derive(Debug)]
pub struct JsonLinesField {
    pub path: String,
    pub occurrences: usize,
    pub types: Vec<&'static str>,
    pub example: Option<String>,
}

#[derive(Debug, Default)]
pub struct JsonLinesFieldCatalog {
    pub fields: Vec<JsonLinesField>,
    pub valid_records: usize,
    pub invalid_records: usize,
    pub truncated: bool,
}

/// Discover object paths across the file. Arrays are kept as a field, without
/// turning individual array positions into columns.
pub fn json_lines_fields(path: &Path) -> Result<JsonLinesFieldCatalog, String> {
    const FIELD_LIMIT: usize = 2048;
    const PATH_LIMIT: usize = 255;
    fn object(
        values: &serde_json::Map<String, Value>,
        parent: &mut String,
        fields: &mut BTreeMap<String, JsonLinesField>,
        truncated: &mut bool,
    ) {
        for (key, value) in values {
            let length = parent.len();
            parent.push('/');
            parent.push_str(&key.replace('~', "~0").replace('/', "~1"));
            if parent.encode_utf16().take(PATH_LIMIT + 1).count() > PATH_LIMIT
                || parent.contains('\0')
                || (!fields.contains_key(parent.as_str()) && fields.len() == FIELD_LIMIT)
            {
                *truncated = true;
                parent.truncate(length);
                continue;
            }
            let field = fields
                .entry(parent.clone())
                .or_insert_with(|| JsonLinesField {
                    path: parent.clone(),
                    occurrences: 0,
                    types: Vec::new(),
                    example: None,
                });
            field.occurrences += 1;
            let kind = match value {
                Value::Null => "null",
                Value::Bool(_) => "boolean",
                Value::Number(_) => "number",
                Value::String(_) => "string",
                Value::Array(_) => "array",
                Value::Object(_) => "object",
            };
            if !field.types.contains(&kind) {
                field.types.push(kind);
            }
            if field.example.is_none() && !value.is_null() {
                let value = value.to_string();
                let mut chars = value.chars();
                let mut example: String = chars.by_ref().take(100).collect();
                if chars.next().is_some() {
                    example.push('…');
                }
                field.example = Some(example);
            }
            if let Value::Object(nested) = value {
                object(nested, parent, fields, truncated);
            }
            parent.truncate(length);
        }
    }

    let file = File::open(path).map_err(|error| error.to_string())?;
    let before = file.metadata().map_err(|error| error.to_string())?;
    let max_bytes = usize::try_from(before.len()).map_err(|_| "JSON Lines file is too large")?;
    let mut catalog = JsonLinesFieldCatalog::default();
    let mut fields = BTreeMap::new();
    scan_lines(BufReader::new(&file), max_bytes, true, |line, _| {
        match line.and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok()) {
            Some(Value::Object(values)) => {
                catalog.valid_records += 1;
                object(
                    &values,
                    &mut String::new(),
                    &mut fields,
                    &mut catalog.truncated,
                );
            }
            _ => catalog.invalid_records += 1,
        }
        Ok(())
    })?;
    let after = file.metadata().map_err(|error| error.to_string())?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err("JSON Lines file changed during import".into());
    }
    catalog.fields = fields.into_values().collect();
    Ok(catalog)
}

fn recover_cell(raw: &RawValue) -> Result<(CellValue, Option<QType>), String> {
    match cell(raw) {
        Err(error) if error == PRECISION_ERROR => {
            check_text_limit(raw.get())?;
            Ok((CellValue::Text(raw.get().into()), Some(QType::Text)))
        }
        result => result,
    }
}

/// Visit a recoverable dataset. Invalid records remain addressable by physical line.
pub(super) fn visit_recoverable_json_lines(
    path: &Path,
    selected: &[String],
    pointers: bool,
    max_lines: usize,
    schema_only: bool,
    mut visit: impl FnMut(&[QColumn], Option<Vec<CellValue>>) -> Result<(), String>,
) -> Result<JsonLinesSamplePosition, String> {
    const LINE: &str = "_jsonl_line";
    const ERROR: &str = "_jsonl_error";
    let mut input = BufReader::new(File::open(path).map_err(|error| error.to_string())?);
    let before = input
        .get_ref()
        .metadata()
        .map_err(|error| error.to_string())?;
    let max_bytes = usize::try_from(before.len()).map_err(|_| "JSON Lines file is too large")?;
    let mut indices = HashMap::new();
    let mut columns = Vec::new();
    let mut types = Vec::new();
    for name in [LINE, ERROR]
        .into_iter()
        .chain(selected.iter().map(String::as_str))
    {
        add_sql_column(name, &mut indices, &mut columns, &mut types)?;
    }
    let reserved = |name: &str| name.eq_ignore_ascii_case(LINE) || name.eq_ignore_ascii_case(ERROR);
    if selected.iter().any(|name| reserved(name)) {
        return Err("JSON Lines recovery reserves _jsonl_line and _jsonl_error".into());
    }

    let selected = selected.iter().map(String::as_str).collect::<HashSet<_>>();
    if pointers
        && (selected.is_empty()
            || selected.iter().any(|path| {
                !path.starts_with('/')
                    || path.contains('\0')
                    || path
                        .split('~')
                        .skip(1)
                        .any(|part| !part.starts_with(['0', '1']))
            }))
    {
        return Err("Select nonempty JSON Pointer paths starting with /".into());
    }
    let parse =
        |line: Option<&[u8]>| -> Result<BTreeMap<String, (CellValue, Option<QType>)>, String> {
            let line = line.ok_or("JSON Lines record exceeds 1 MiB")?;
            if pointers {
                let root: Value = serde_json::from_slice(line)
                    .map_err(|error| format!("Expected one JSON object ({error})"))?;
                if !root.is_object() {
                    return Err("Expected one JSON object".into());
                }
                return selected
                    .iter()
                    .filter_map(|&path| root.pointer(path).map(|value| (path, value)))
                    .map(|(path, value)| {
                        let raw = serde_json::value::to_raw_value(value)
                            .map_err(|error| error.to_string())?;
                        Ok((path.into(), recover_cell(&raw)?))
                    })
                    .collect();
            }
            let raw: BTreeMap<String, &RawValue> = serde_json::from_slice(line)
                .map_err(|error| format!("Expected one JSON object ({error})"))?;
            raw.into_iter()
                .filter(|(name, _)| selected.is_empty() || selected.contains(name.as_str()))
                .map(|(name, value)| Ok((name, recover_cell(value)?)))
                .collect()
        };
    let position = scan_lines_up_to(&mut input, max_bytes, true, max_lines, |line, _| {
        if let Ok(fields) = parse(line) {
            for (name, (_, ty)) in fields {
                if reserved(&name) {
                    return Err("JSON Lines recovery reserves _jsonl_line and _jsonl_error".into());
                }
                let index = match indices.get(&name) {
                    Some(&index) => index,
                    None => add_sql_column(&name, &mut indices, &mut columns, &mut types)?,
                };
                if let Some(ty) = ty {
                    types[index] = Some(merge_type(types[index], ty));
                }
            }
        }
        Ok(())
    })?;
    types[0] = Some(QType::Whole);
    types[1] = Some(QType::Text);
    for (column, ty) in columns.iter_mut().zip(types) {
        column.ty = ty.unwrap_or(QType::Any);
    }
    visit(&columns, None)?;
    if !schema_only {
        input.rewind().map_err(|error| error.to_string())?;
        let read_position =
            scan_lines_up_to(&mut input, max_bytes, true, max_lines, |line, number| {
                let mut row = vec![CellValue::Empty; columns.len()];
                row[0] = CellValue::Number(number as f64);
                match parse(line) {
                    Ok(fields) => {
                        for (name, (value, _)) in fields {
                            let index = indices
                                .get(&name)
                                .copied()
                                .filter(|index| *index >= 2)
                                .ok_or("JSON Lines schema changed during import")?;
                            row[index] = value;
                        }
                    }
                    Err(error) => row[1] = CellValue::Text(error),
                }
                visit(&columns, Some(row))
            })?;
        if read_position != position {
            return Err("JSON Lines file changed during import".into());
        }
    }
    let after = input
        .get_ref()
        .metadata()
        .map_err(|error| error.to_string())?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err("JSON Lines file changed during import".into());
    }
    Ok(position)
}

fn infer_type(
    previous: &mut Option<QType>,
    raw: &RawValue,
    line: usize,
    name: &str,
) -> Result<(), String> {
    let ty = inferred_cell_type(raw).map_err(|error| format!("Line {line}, {name}: {error}"))?;
    if let Some(ty) = ty {
        *previous = Some(merge_type(*previous, ty));
    }
    Ok(())
}

fn inferred_cell_type(raw: &RawValue) -> Result<Option<QType>, String> {
    let text = raw.get();
    let borrowed = if text.starts_with(['[', '{']) {
        Some(text)
    } else if text.starts_with('"') {
        // Unescaped strings can be validated without allocating their decoded value.
        serde_json::from_str::<&str>(text).ok()
    } else {
        None
    };
    if let Some(text) = borrowed {
        check_text_limit(text)?;
        return Ok(Some(QType::Text));
    }
    cell(raw).map(|(_, ty)| ty)
}

fn sql_cell(raw: Option<&RawValue>, line: usize, name: &str) -> Result<CellValue, String> {
    raw.map_or(Ok(CellValue::Empty), |raw| {
        cell(raw)
            .map(|(value, _)| value)
            .map_err(|error| format!("Line {line}, {name}: {error}"))
    })
}

fn visit_projected_lines(
    reader: impl BufRead,
    max_bytes: usize,
    indices: &HashMap<String, usize>,
    mut visit: impl FnMut(Vec<Option<&RawValue>>, usize) -> Result<(), String>,
) -> Result<(), String> {
    visit_raw_lines(reader, max_bytes, |line, number| {
        let mut parser = serde_json::Deserializer::from_slice(line);
        let fields = Projection(indices)
            .deserialize(&mut parser)
            .and_then(|fields| {
                parser.end()?;
                Ok(fields)
            })
            .map_err(|error| record_error(number, error))?;
        visit(fields, number)
    })
}

#[derive(Deserialize)]
#[serde(transparent)]
struct FieldName<'a>(#[serde(borrow)] Cow<'a, str>);

struct Projection<'a>(&'a HashMap<String, usize>);

impl<'de> DeserializeSeed<'de> for Projection<'_> {
    type Value = Vec<Option<&'de RawValue>>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for Projection<'_> {
    type Value = Vec<Option<&'de RawValue>>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut fields = vec![None; self.0.len()];
        while let Some(FieldName(name)) = map.next_key()? {
            let value = map.next_value()?;
            if let Some(&index) = self.0.get(name.as_ref()) {
                // Match BTreeMap deserialization: the last duplicate key wins.
                fields[index] = Some(value);
            }
        }
        Ok(fields)
    }
}

fn add_sql_column(
    name: &str,
    indices: &mut HashMap<String, usize>,
    columns: &mut Vec<QColumn>,
    types: &mut Vec<Option<QType>>,
) -> Result<usize, String> {
    let index = columns.len();
    if index >= 256 || name.is_empty() || exceeds_text_limit(name) || indices.contains_key(name) {
        return Err(format!(
            "Invalid, duplicate or excess JSON Lines column: {name}"
        ));
    }
    indices.insert(name.into(), index);
    columns.push(QColumn {
        name: name.into(),
        ty: QType::Any,
    });
    types.push(None);
    Ok(index)
}

#[derive(Default)]
struct Records {
    table: QTable,
    indices: HashMap<String, usize>,
    types: Vec<Option<QType>>,
}

impl Records {
    fn push(&mut self, fields: BTreeMap<String, &RawValue>, location: &str) -> Result<(), String> {
        let width = self.indices.len()
            + fields
                .keys()
                .filter(|key| !self.indices.contains_key(*key))
                .count();
        let height = self.table.rows.len() + 1;
        if width > MAX_COLUMNS || height > MAX_DATA_ROWS || width.saturating_mul(height) > MAX_CELLS
        {
            return Err(format!(
                "{location}: JSON source exceeds the table size limit"
            ));
        }
        let mut row = vec![CellValue::Empty; width];
        for (name, value) in fields {
            if exceeds_text_limit(&name) {
                return Err(format!(
                    "{location}: field name exceeds the spreadsheet cell limit"
                ));
            }
            let index = *self.indices.entry(name.clone()).or_insert_with(|| {
                let index = self.table.columns.len();
                self.table.columns.push(QColumn {
                    name: name.clone(),
                    ty: QType::Any,
                });
                self.types.push(None);
                index
            });
            let (cell, ty) = cell(value).map_err(|error| format!("{location}, {name}: {error}"))?;
            if let Some(ty) = ty {
                self.types[index] = Some(merge_type(self.types[index], ty));
            }
            row[index] = cell;
        }
        self.table.rows.push(row);
        Ok(())
    }

    fn finish(mut self) -> Result<QTable, String> {
        if self.table.columns.is_empty() {
            return Err("JSON source contains no fields".into());
        }
        for (column, ty) in self.table.columns.iter_mut().zip(self.types) {
            column.ty = ty.unwrap_or(QType::Any);
        }
        for row in &mut self.table.rows {
            row.resize(self.table.columns.len(), CellValue::Empty);
        }
        Ok(self.table)
    }
}

fn json_bytes(path: &Path) -> Result<Vec<u8>, String> {
    if !path.is_file() {
        return Err("Select a local JSON file".into());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| {
            file.take((MAX_SOURCE_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
        })
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err("JSON source exceeds the byte limit".into());
    }
    Ok(bytes)
}

/// The root array, or the names of array-valued fields in the root object.
pub fn json_array_choices(path: &Path) -> Result<Vec<Option<String>>, String> {
    let bytes = json_bytes(path)?;
    let document: &RawValue = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if document.get().starts_with('[') {
        return Ok(vec![None]);
    }
    let fields: BTreeMap<String, &RawValue> =
        serde_json::from_str(document.get()).map_err(|error| error.to_string())?;
    let choices: Vec<_> = fields
        .into_iter()
        .filter(|(_, value)| value.get().starts_with('['))
        .map(|(key, _)| Some(key))
        .collect();
    if choices.len() > 1024 {
        return Err("JSON source has too many arrays to list".into());
    }
    Ok(choices)
}

pub fn read_json_array(path: &Path, key: Option<&str>) -> Result<QTable, String> {
    let bytes = json_bytes(path)?;
    records_from_array(&bytes, key)
}

fn records_from_array(bytes: &[u8], key: Option<&str>) -> Result<QTable, String> {
    let document: &RawValue = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    let array = match key {
        Some(key) => {
            let fields: BTreeMap<String, &RawValue> =
                serde_json::from_str(document.get()).map_err(|error| error.to_string())?;
            *fields
                .get(key)
                .ok_or_else(|| format!("Missing JSON array: {key}"))?
        }
        None => document,
    };
    let mut remaining = array
        .get()
        .strip_prefix('[')
        .and_then(|text| text.strip_suffix(']'))
        .ok_or("Expected a JSON array")?
        .trim_start();
    let mut output = Records::default();
    // RawValue has already validated the array syntax. Read one record at a
    // time so a dense array cannot allocate a second array of references.
    while !remaining.is_empty() {
        let location = format!("Record {}", output.table.rows.len() + 1);
        let mut stream = serde_json::Deserializer::from_str(remaining).into_iter::<&RawValue>();
        let row = stream
            .next()
            .ok_or("Expected a JSON record")?
            .map_err(|error| format!("{location}: {error}"))?;
        let fields = serde_json::from_str(row.get())
            .map_err(|error| format!("{location}: expected an object ({error})"))?;
        output.push(fields, &location)?;
        remaining = remaining[stream.byte_offset()..].trim_start();
        if let Some(rest) = remaining.strip_prefix(',') {
            remaining = rest.trim_start();
        }
    }
    output.finish()
}

fn cell(raw: &RawValue) -> Result<(CellValue, Option<QType>), String> {
    let text = raw.get();
    if text.starts_with(['[', '{']) {
        check_text_limit(text)?;
        return Ok((CellValue::Text(text.into()), Some(QType::Text)));
    }
    let value = serde_json::from_str::<Value>(text).map_err(|error| error.to_string())?;
    let (cell, ty) = match value {
        Value::Null => return Ok((CellValue::Empty, None)),
        Value::Bool(value) => (CellValue::Bool(value), QType::Logical),
        Value::String(value) => (CellValue::Text(value), QType::Text),
        Value::Number(value) => {
            let number = value.as_f64().ok_or(PRECISION_ERROR)?;
            if number.abs() >= (1u64 << 53) as f64 && number.fract() == 0. {
                return Err(PRECISION_ERROR.into());
            }
            let ty = if value.is_i64() || value.is_u64() {
                QType::Whole
            } else {
                QType::Decimal
            };
            (CellValue::Number(number), ty)
        }
        Value::Array(_) | Value::Object(_) => return Err("Expected a scalar JSON value".into()),
    };
    if let CellValue::Text(text) = &cell {
        check_text_limit(text)?;
    }
    Ok((cell, Some(ty)))
}

fn check_text_limit(text: &str) -> Result<(), String> {
    if exceeds_text_limit(text) {
        return Err("Text exceeds the spreadsheet cell limit".into());
    }
    Ok(())
}

fn exceeds_text_limit(text: &str) -> bool {
    text.encode_utf16().take(MAX_TEXT_UNITS + 1).count() > MAX_TEXT_UNITS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_position_distinguishes_estimates_from_exact_end() {
        let input = b"{}\r\n{invalid\n{}";
        let scan =
            |limit| scan_lines_up_to(&input[..], input.len(), true, limit, |_, _| Ok(())).unwrap();
        let prefix = scan(1);
        assert_eq!(
            (prefix.lines, prefix.bytes, prefix.file_bytes),
            (1, 4, input.len())
        );
        assert!(!prefix.at_end());
        assert_eq!(prefix.estimated_lines(), input.len() / 4);
        let exact = scan(3);
        assert!(exact.at_end());
        assert_eq!(
            (exact.lines, exact.bytes, exact.estimated_lines()),
            (3, input.len(), 3)
        );
        let empty = scan_lines_up_to(&[][..], 0, true, 200, |_, _| Ok(())).unwrap();
        assert!(empty.at_end());
        assert_eq!(empty.estimated_lines(), 0);
        let estimate = JsonLinesSamplePosition {
            lines: 200,
            bytes: 40 * 1024,
            file_bytes: 20 * 1024 * 1024,
        };
        assert_eq!(estimate.estimated_lines(), 100_000);
    }

    #[test]
    fn imports_sparse_records_with_new_fields_and_mixed_types() {
        let table = read_records(&b"{\"amount\":2,\"code\":\"001\",\"tags\":[true]}\r\n{\"amount\":2.5,\"extra\":false,\"tags\":null}\n{\"code\":9,\"tags\":{\"a\":18446744073709551617}}"[..]).unwrap();
        assert_eq!(table.names(), ["amount", "code", "tags", "extra"]);
        assert_eq!(
            table.columns.iter().map(|col| col.ty).collect::<Vec<_>>(),
            [QType::Decimal, QType::Any, QType::Text, QType::Logical]
        );
        assert_eq!(
            table.rows[0],
            [
                CellValue::Number(2.),
                CellValue::Text("001".into()),
                CellValue::Text("[true]".into()),
                CellValue::Empty
            ]
        );
        assert_eq!(
            table.rows[1],
            [
                CellValue::Number(2.5),
                CellValue::Empty,
                CellValue::Empty,
                CellValue::Bool(false)
            ]
        );
        assert_eq!(
            table.rows[2][2],
            CellValue::Text("{\"a\":18446744073709551617}".into())
        );
    }

    #[test]
    fn projected_stream_preserves_keys_values_and_validation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("projection.jsonl");
        let selected = ["code", "n", "nested"].map(String::from);
        let source = super::super::DiskDataset::JsonLines(path.clone(), selected.to_vec());
        let valid = r#"{"code":"discarded","c\u006fde":"001","n":2,"nested":{"x": 1},"ignored":[0]}
{"code":"002","n":2.5}
"#;
        std::fs::write(&path, valid).unwrap();
        let table = source.collect_bounded(20, 1024).unwrap();
        assert_eq!(table.names(), ["code", "n", "nested"]);
        assert_eq!(table.columns[1].ty, QType::Decimal);
        assert_eq!(
            table.rows,
            [
                [
                    CellValue::Text("001".into()),
                    CellValue::Number(2.0),
                    CellValue::Text(r#"{"x": 1}"#.into())
                ],
                [
                    CellValue::Text("002".into()),
                    CellValue::Number(2.5),
                    CellValue::Empty
                ],
            ]
        );
        for invalid in [
            r#"{"ignored":[,]}"#,
            r#"{"n":9007199254740993}"#,
            r#"{"code":"003"} {}"#,
        ] {
            std::fs::write(&path, format!("{valid}{invalid}\n")).unwrap();
            let error = source.columns().unwrap_err();
            assert!(
                error.starts_with("Line 3:") || error.starts_with("Line 3,"),
                "{error}"
            );
        }
    }

    #[test]
    fn catalog_and_projections_recover_at_the_next_physical_line() {
        use super::super::DiskDataset;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recovery.ndjson");
        let prefix = b"{\"event\":\"before\",\"nested\":{\"id\":1}}\r\n";
        // A valid last record need not have a newline.
        let suffix = b"{\"event\":\"after\",\"nested\":{\"id\":2,\"late\":true}}";
        let at_limit = [b"{\"discarded\":\"".as_slice(), &vec![b'x'; MAX_LINE_BYTES]].concat();
        let mut oversized_with_delimiter = at_limit.clone();
        oversized_with_delimiter.truncate(MAX_LINE_BYTES);
        oversized_with_delimiter.push(b'\n');

        for (label, broken, expected) in [
            ("blank", b"  \r\n".as_slice(), "Expected one JSON object"),
            (
                "truncated object",
                b"{\"event\":\n",
                "Expected one JSON object",
            ),
            (
                "trailing value",
                b"{\"event\":\"discarded\"} {}\n",
                "Expected one JSON object",
            ),
            (
                "array",
                b"[{\"event\":\"discarded\"}]\n",
                "Expected one JSON object",
            ),
            ("null", b"null\n", "Expected one JSON object"),
            (
                "invalid UTF-8",
                b"{\"event\":\"\xff\"}\n",
                "Expected one JSON object",
            ),
            (
                "oversized without delimiter in buffer",
                &[at_limit.as_slice(), b"\r\n"].concat(),
                "JSON Lines record exceeds 1 MiB",
            ),
            (
                "oversized ending with delimiter in buffer",
                oversized_with_delimiter.as_slice(),
                "JSON Lines record exceeds 1 MiB",
            ),
        ] {
            std::fs::write(&path, [prefix, broken, suffix].concat()).unwrap();
            let catalog = json_lines_fields(&path).unwrap();
            assert_eq!(
                (catalog.valid_records, catalog.invalid_records),
                (2, 1),
                "{label}"
            );
            let late = catalog
                .fields
                .iter()
                .find(|field| field.path == "/nested/late")
                .unwrap();
            assert_eq!(late.occurrences, 1, "{label}");

            for dataset in [
                DiskDataset::RecoverableJsonLines(path.clone(), vec!["event".into()]),
                DiskDataset::JsonLinesPaths(path.clone(), vec!["/event".into()]),
            ] {
                let rows = dataset.collect_bounded(32, 2048).unwrap().rows;
                assert_eq!(rows.len(), 3, "{label}");
                for (index, row) in rows.iter().enumerate() {
                    assert_eq!(row[0], CellValue::Number((index + 1) as f64), "{label}");
                }
                assert_eq!(
                    rows[0][1..],
                    [CellValue::Empty, CellValue::Text("before".into())],
                    "{label}"
                );
                assert!(
                    matches!(&rows[1][1], CellValue::Text(error) if error.starts_with(expected)),
                    "{label}: {:?}",
                    rows[1]
                );
                assert_eq!(rows[1][2], CellValue::Empty, "{label}");
                assert_eq!(
                    rows[2][1..],
                    [CellValue::Empty, CellValue::Text("after".into())],
                    "{label}"
                );
            }
        }
    }

    #[test]
    fn recovery_reports_a_cell_limit_only_when_the_field_is_selected() {
        use super::super::DiskDataset;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("text-limit.jsonl");
        let input = format!(
            "{{\"event\":\"first\",\"detail\":\"{}\"}}\n{{\"event\":\"next\",\"detail\":\"ok\"}}",
            "x".repeat(MAX_TEXT_UNITS + 1),
        );
        std::fs::write(&path, input).unwrap();
        let catalog = json_lines_fields(&path).unwrap();
        assert_eq!((catalog.valid_records, catalog.invalid_records), (2, 0));

        let events = DiskDataset::JsonLinesPaths(path.clone(), vec!["/event".into()])
            .collect_bounded(16, 256)
            .unwrap();
        assert_eq!(
            events.rows[0][1..],
            [CellValue::Empty, CellValue::Text("first".into())]
        );

        let details = DiskDataset::JsonLinesPaths(path, vec!["/detail".into()])
            .collect_bounded(16, 256)
            .unwrap();
        assert_eq!(
            details.rows[0],
            [
                CellValue::Number(1.0),
                CellValue::Text("Text exceeds the spreadsheet cell limit".into()),
                CellValue::Empty,
            ]
        );
        assert_eq!(
            details.rows[1],
            [
                CellValue::Number(2.0),
                CellValue::Empty,
                CellValue::Text("ok".into()),
            ]
        );
    }

    #[test]
    fn rejects_invalid_records_and_precision_loss_with_line_numbers() {
        for record in [
            "[]",
            "{bad}",
            "",
            "{\"n\":9007199254740993}",
            "{\"n\":18446744073709551617}",
        ] {
            let data = format!("{{\"n\":1}}\n{record}\n");
            assert!(
                read_records(data.as_bytes())
                    .unwrap_err()
                    .starts_with("Line 2")
            );
        }
        assert!(
            read_records(vec![b' '; MAX_LINE_BYTES + 1].as_slice())
                .unwrap_err()
                .contains("byte limit")
        );
        assert!(read_records(&b"{}"[..]).is_err());
    }
}

#[cfg(test)]
#[test]
fn chooses_json_array_and_preserves_record_types() {
    let bytes = br#"{"packages":[{"code":"001","n":2},{"n":2.5,"extra":true}],"items":[1]}"#;
    let table = records_from_array(bytes, Some("packages")).unwrap();
    assert_eq!(table.names(), ["code", "n", "extra"]);
    assert_eq!(table.rows[0][0], CellValue::Text("001".into()));
    assert_eq!(table.rows[1][0], CellValue::Empty);
    assert_eq!(table.columns[1].ty, QType::Decimal);
    assert!(records_from_array(bytes, Some("items")).is_err());
    assert!(records_from_array(br#"[{"n":1}]"#, None).is_ok());
}
