// SPDX-License-Identifier: AGPL-3.0-only
use super::{QColumn, QTable, QType};
use crate::CellValue;
use parquet::basic::{ColumnOrder, ConvertedType, LogicalType, Repetition, SortOrder, Type as PhysicalType};
use parquet::file::metadata::RowGroupMetaData;
use parquet::file::statistics::Statistics;
use parquet::file::reader::{FileReader, SerializedFileReader};
use parquet::record::Field;
use parquet::schema::types::Type;
use std::{
    fs::File,
    path::{Path, PathBuf},
};

fn number(value: f64, ty: QType) -> Result<(CellValue, QType), String> {
    if !value.is_finite() {
        return Err("Non-finite Parquet number".into());
    }
    Ok((CellValue::Number(value), ty))
}

fn integer(value: i128) -> Result<(CellValue, QType), String> {
    if value.unsigned_abs() >= 1 << 53 {
        return Err("Parquet integer exceeds spreadsheet precision".into());
    }
    number(value as f64, QType::Whole)
}

fn cell(field: Field) -> Result<(CellValue, QType), String> {
    match field {
        Field::Null => Ok((CellValue::Empty, QType::Any)),
        Field::Bool(v) => Ok((CellValue::Bool(v), QType::Logical)),
        Field::Str(v) => Ok((CellValue::Text(v), QType::Text)),
        Field::Byte(v) => integer(v.into()),
        Field::Short(v) => integer(v.into()),
        Field::Int(v) => integer(v.into()),
        Field::Long(v) => integer(v.into()),
        Field::UByte(v) => integer(v.into()),
        Field::UShort(v) => integer(v.into()),
        Field::UInt(v) => integer(v.into()),
        Field::ULong(v) => integer(v.into()),
        Field::Float16(v) => number(v.to_f64(), QType::Decimal),
        Field::Float(v) => number(v.into(), QType::Decimal),
        Field::Double(v) => number(v, QType::Decimal),
        Field::Date(v) => number(f64::from(v) + 25_569., QType::Date),
        Field::TimeMillis(v) => number(f64::from(v) / 86_400_000., QType::Time),
        Field::TimeMicros(v) => number(v as f64 / 86_400_000_000., QType::Time),
        Field::TimestampMillis(v) => number(v as f64 / 86_400_000. + 25_569., QType::DateTime),
        Field::TimestampMicros(v) => number(v as f64 / 86_400_000_000. + 25_569., QType::DateTime),
        _ => Err("Unsupported Parquet value type".into()),
    }
}

// Recognize the standard three-level LIST encoding for UTF-8 text.
fn is_text_list(field: &Type) -> bool {
    if field.is_primitive()
        || field.get_basic_info().converted_type() != ConvertedType::LIST
        || field.get_basic_info().repetition() == Repetition::REPEATED
    {
        return false;
    }
    let [entries] = field.get_fields() else { return false };
    if entries.is_primitive()
        || entries.name() != "list"
        || entries.get_basic_info().repetition() != Repetition::REPEATED
    {
        return false;
    }
    let [element] = entries.get_fields() else { return false };
    element.is_primitive()
        && element.get_physical_type() == PhysicalType::BYTE_ARRAY
        && element.get_basic_info().converted_type() == ConvertedType::UTF8
        && element.get_basic_info().repetition() != Repetition::REPEATED
}

fn text_list_cell(field: Field, max_text: usize) -> Result<CellValue, String> {
    use std::io::{self, Write};

    struct BoundedJson {
        bytes: Vec<u8>,
        limit: usize,
    }
    impl Write for BoundedJson {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            if data.len() > self.limit - self.bytes.len() {
                return Err(io::Error::other("Parquet text exceeds the import limit"));
            }
            self.bytes.extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let list = match field {
        Field::Null => return Ok(CellValue::Empty),
        Field::ListInternal(list) => list,
        _ => return Err("Expected a Parquet text list".into()),
    };
    if list.elements().len() > max_text {
        return Err("Parquet text exceeds the import limit".into());
    }
    let values = list.elements().iter().map(|item| match item {
        Field::Str(value) => Ok(Some(value.as_str())),
        Field::Null => Ok(None),
        _ => Err("Unsupported Parquet text list element"),
    }).collect::<Result<Vec<_>, _>>()?;
    let mut output = BoundedJson { bytes: Vec::new(), limit: max_text };
    serde_json::to_writer(&mut output, &values).map_err(|error| error.to_string())?;
    let json = String::from_utf8(output.bytes).map_err(|error| error.to_string())?;
    Ok(CellValue::Text(json.into()))
}

/// A top-level field and its spreadsheet type, if the reader can import it.
pub struct ParquetColumn {
    pub name: String,
    pub ty: Option<QType>,
}

fn column_type(field: &Type) -> Option<QType> {
    let info = field.get_basic_info();
    if !field.is_primitive() || info.repetition() == Repetition::REPEATED {
        return None;
    }
    use ConvertedType as C;
    use PhysicalType as P;
    Some(match (field.get_physical_type(), info.converted_type()) {
        (P::BOOLEAN, C::NONE) => QType::Logical,
        (P::FLOAT | P::DOUBLE, C::NONE) => QType::Decimal,
        (P::FIXED_LEN_BYTE_ARRAY, C::NONE)
            if info.logical_type_ref() == Some(&LogicalType::Float16) =>
        {
            QType::Decimal
        }
        (P::BYTE_ARRAY, C::UTF8 | C::ENUM | C::JSON) => QType::Text,
        (P::INT32, C::DATE) => QType::Date,
        (P::INT32, C::TIME_MILLIS) | (P::INT64, C::TIME_MICROS) => QType::Time,
        (P::INT96, C::NONE) | (P::INT64, C::TIMESTAMP_MILLIS | C::TIMESTAMP_MICROS) => {
            QType::DateTime
        }
        (P::INT32 | P::INT64, C::NONE) if info.logical_type_ref().is_none() => QType::Whole,
        (P::INT32, C::INT_8 | C::INT_16 | C::INT_32 | C::UINT_8 | C::UINT_16 | C::UINT_32)
        | (P::INT64, C::INT_64 | C::UINT_64) => QType::Whole,
        _ => return None,
    })
}

fn open(path: &Path) -> Result<SerializedFileReader<File>, String> {
    if !path.is_file() {
        return Err("Select a local Parquet file".into());
    }
    let file = File::open(path).map_err(|e| e.to_string())?;
    SerializedFileReader::new(file).map_err(|e| e.to_string())
}

/// Read only the schema, without decoding column data.
pub fn parquet_columns(path: &Path) -> Result<Vec<ParquetColumn>, String> {
    let reader = open(path)?;
    Ok(reader
        .metadata()
        .file_metadata()
        .schema()
        .get_fields()
        .iter()
        .map(|field| ParquetColumn {
            name: field.name().into(),
            ty: column_type(field),
        })
        .collect())
}

/// List the first file's fields, allowing only names with a common type across the selection.
pub fn parquet_files_columns(paths: &[PathBuf]) -> Result<Vec<ParquetColumn>, String> {
    let mut paths = paths.iter();
    let first = paths.next().ok_or("Select at least one Parquet file")?;
    let mut columns = parquet_columns(first)?;
    for path in paths {
        let fields =
            parquet_columns(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let mut by_name = std::collections::HashMap::new();
        for field in fields {
            by_name
                .entry(field.name)
                .and_modify(|ty| *ty = None)
                .or_insert(field.ty);
        }
        for column in &mut columns {
            if by_name.get(&column.name).copied().flatten() != column.ty {
                column.ty = None;
            }
        }
    }
    Ok(columns)
}

/// Read a set of files into a single table with a common projected schema.
pub fn read_parquet_files(paths: &[PathBuf], selected: &[String]) -> Result<QTable, String> {
    if paths.is_empty() {
        return Err("Select at least one Parquet file".into());
    }
    let mut table = QTable::default();
    let mut seen = std::collections::HashSet::new();
    for (index, path) in paths.iter().enumerate() {
        let canonical = path
            .canonicalize()
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if !seen.insert(canonical) {
            return Err(format!("Duplicate Parquet file: {}", path.display()));
        }
        let expected = (index != 0).then_some(table.columns.as_slice());
        let mut part = read_columns(path, selected, table.rows.len(), expected)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if index == 0 {
            table.columns = part.columns;
        }
        table.rows.append(&mut part.rows);
    }
    Ok(table)
}

/// Locate Parquet files below a selected folder without following symlinks.
pub fn parquet_folder_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut directories = vec![root.to_path_buf()];
    let mut files = Vec::new();
    let mut visited = 0;
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(&directory).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            visited += 1;
            if visited > 16_384 {
                return Err("Parquet folder contains too many entries".into());
            }
            let kind = entry.file_type().map_err(|error| error.to_string())?;
            let path = entry.path();
            if entry.file_name().to_string_lossy().starts_with(['.', '_']) {
                continue;
            }
            if kind.is_dir() {
                directories.push(path);
            } else if kind.is_file()
                && path
                    .extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value.eq_ignore_ascii_case("parquet"))
            {
                files.push(path);
            }
        }
    }
    files.sort();
    if files.is_empty() {
        return Err("No Parquet files found in the selected folder".into());
    }
    Ok(files)
}

fn hive_partition(value: &str) -> Result<String, String> {
    let mut chars = value.bytes();
    let mut result = Vec::with_capacity(value.len());
    while let Some(ch) = chars.next() {
        if ch == b'%' {
            let digit = |value: Option<u8>| value.and_then(|value| char::from(value).to_digit(16));
            let hi = digit(chars.next()).ok_or("Invalid escape in Parquet partition name")?;
            let lo = digit(chars.next()).ok_or("Invalid escape in Parquet partition name")?;
            result.push(((hi << 4) | lo) as u8);
        } else {
            result.push(ch);
        }
    }
    String::from_utf8(result).map_err(|error| error.to_string())
}

fn hive_values(
    root: &Path,
    file: &Path,
) -> Result<std::collections::BTreeMap<String, CellValue>, String> {
    let mut values = std::collections::BTreeMap::new();
    let relative = file.strip_prefix(root).map_err(|error| error.to_string())?;
    for component in relative.parent().unwrap_or(Path::new("")).components() {
        let component = component
            .as_os_str()
            .to_str()
            .ok_or("Invalid UTF-8 in Parquet folder name")?;
        let Some((key, value)) = component.split_once('=') else {
            continue;
        };
        let key = hive_partition(key)?;
        let value = hive_partition(value)?;
        if key.is_empty() || values.contains_key(&key) {
            return Err(format!("Invalid or duplicate Parquet partition key: {key}"));
        }
        values.insert(
            key,
            if value == "__HIVE_DEFAULT_PARTITION__" {
                CellValue::Empty
            } else {
                CellValue::Text(value)
            },
        );
    }
    Ok(values)
}

/// Read common data columns and append the Hive partition columns as text.
pub fn read_parquet_folder(root: &Path, selected: &[String]) -> Result<QTable, String> {
    let files = parquet_folder_files(root)?;
    let partitions = files
        .iter()
        .map(|file| hive_values(root, file))
        .collect::<Result<Vec<_>, _>>()?;
    let keys: std::collections::BTreeSet<_> = partitions
        .iter()
        .flat_map(|values| values.keys().cloned())
        .collect();
    let mut result = QTable::default();
    let mut data_width = 0;
    for (index, (file, mut values)) in files.iter().zip(partitions).enumerate() {
        let expected = (index != 0).then_some(&result.columns[..data_width]);
        let mut part = read_columns(file, selected, result.rows.len(), expected)
            .map_err(|error| format!("{}: {error}", file.display()))?;
        if index == 0 {
            data_width = part.columns.len();
            if part
                .columns
                .iter()
                .any(|column| keys.contains(&column.name))
            {
                return Err("A Parquet partition column also occurs in the file schema".into());
            }
            result.columns = part.columns;
            result
                .columns
                .extend(keys.iter().cloned().map(|name| QColumn {
                    name,
                    ty: QType::Text,
                }));
        }
        let width = result.columns.len();
        if width > 16_384 || width.saturating_mul(result.rows.len() + part.rows.len()) > 5_000_000 {
            return Err("Parquet folder exceeds the query editor's size limit".into());
        }
        let partition: Vec<_> = keys
            .iter()
            .map(|key| values.remove(key).unwrap_or(CellValue::Empty))
            .collect();
        for row in &mut part.rows {
            row.extend(partition.iter().cloned());
        }
        result.rows.append(&mut part.rows);
    }
    Ok(result)
}

pub fn read_parquet(path: &Path) -> Result<QTable, String> {
    read_parquet_columns(path, &[])
}

/// Materialize selected top-level fields. An empty selection means the legacy whole-table source.
pub fn read_parquet_columns(path: &Path, selected: &[String]) -> Result<QTable, String> {
    read_columns(path, selected, 0, None)
}

fn read_columns(
    path: &Path,
    selected: &[String],
    rows_loaded: usize,
    expected: Option<&[QColumn]>,
) -> Result<QTable, String> {
    read_columns_bounded(path, selected, rows_loaded, expected, None, Some((5_000_000, usize::MAX)))
}

pub fn read_parquet_bounded(
    path: &Path,
    selected: &[String],
    max_cells: usize,
    max_text: usize,
) -> Result<QTable, String> {
    read_parquet_filtered(path, selected, &[], max_cells, max_text)
}

pub enum ParquetBound {
    Lt,
    Le,
    Gt,
    Ge,
}

impl ParquetBound {
    fn matches(&self, value: &CellValue, target: &CellValue) -> Option<bool> {
        let order = match (value, target) {
            (CellValue::Text(a), CellValue::Text(b)) => a.cmp(b),
            (CellValue::Number(a), CellValue::Number(b)) => a.partial_cmp(b)?,
            _ => return None,
        };
        Some(match self {
            Self::Lt => order.is_lt(),
            Self::Le => order.is_le(),
            Self::Gt => order.is_gt(),
            Self::Ge => order.is_ge(),
        })
    }
}

pub enum ParquetPredicate {
    Equal(CellValue),
    Range(Vec<(ParquetBound, CellValue)>),
}

impl ParquetPredicate {
    fn accepts_type(&self, ty: QType) -> bool {
        let ordered = |value: &CellValue| matches!((value, ty),
            (CellValue::Text(_), QType::Text)
            | (CellValue::Number(_), QType::Whole | QType::Decimal));
        match self {
            Self::Equal(value) => ordered(value) || matches!((value, ty),
                (CellValue::Empty, _) | (CellValue::Bool(_), QType::Logical)),
            Self::Range(bounds) => !bounds.is_empty() && bounds.iter().all(|(_, value)| ordered(value)),
        }
    }

    fn matches(&self, value: &CellValue) -> bool {
        match self {
            Self::Equal(expected) => expected == value,
            Self::Range(bounds) => bounds.iter().all(|(bound, target)| {
                bound.matches(value, target) == Some(true)
            }),
        }
    }
}

// Only use statistics whose sort order and representation agree with the row matcher.
fn exact_range(stats: &Statistics, order: ColumnOrder, ty: QType) -> Option<(CellValue, CellValue)> {
    if stats.is_min_max_deprecated() || !stats.min_is_exact() || !stats.max_is_exact() {
        return None;
    }
    let (min, max) = match (stats, order, ty) {
        (Statistics::Int32(values), ColumnOrder::TYPE_DEFINED_ORDER(SortOrder::SIGNED), QType::Whole) => {
            let min = *values.min_opt()?;
            let max = *values.max_opt()?;
            (integer(min.into()).ok()?.0, integer(max.into()).ok()?.0)
        }
        (Statistics::Int64(values), ColumnOrder::TYPE_DEFINED_ORDER(SortOrder::SIGNED), QType::Whole) => {
            let min = *values.min_opt()?;
            let max = *values.max_opt()?;
            (integer(min.into()).ok()?.0, integer(max.into()).ok()?.0)
        }
        (Statistics::ByteArray(values), ColumnOrder::TYPE_DEFINED_ORDER(SortOrder::UNSIGNED), QType::Text) => {
            let min = std::str::from_utf8(values.min_opt()?.data()).ok()?;
            let max = std::str::from_utf8(values.max_opt()?.data()).ok()?;
            (CellValue::Text(min.into()), CellValue::Text(max.into()))
        }
        _ => return None,
    };
    (ParquetBound::Le.matches(&min, &max) == Some(true)).then_some((min, max))
}

fn excludes_group(
    group: &RowGroupMetaData,
    index: usize,
    order: ColumnOrder,
    ty: QType,
    predicate: &ParquetPredicate,
) -> bool {
    let Some(stats) = group.column(index).statistics() else { return false };
    if matches!(predicate, ParquetPredicate::Equal(CellValue::Empty)) {
        return stats.null_count_opt() == Some(0);
    }
    if stats.null_count_opt().is_some_and(|count| Some(count) == u64::try_from(group.num_rows()).ok()) {
        return true;
    }
    let Some((min, max)) = exact_range(stats, order, ty) else { return false };
    match predicate {
        ParquetPredicate::Equal(value) => {
            ParquetBound::Ge.matches(&max, value) == Some(false)
                || ParquetBound::Le.matches(&min, value) == Some(false)
        }
        ParquetPredicate::Range(bounds) => bounds.iter().any(|(bound, target)| {
            let endpoint = match bound {
                ParquetBound::Lt | ParquetBound::Le => &min,
                ParquetBound::Gt | ParquetBound::Ge => &max,
            };
            bound.matches(endpoint, target) == Some(false)
        }),
    }
}

/// Scan a projection and retain rows matching all typed conditions.
pub fn read_parquet_filtered(
    path: &Path,
    selected: &[String],
    filters: &[(&str, ParquetPredicate)],
    max_cells: usize,
    max_text: usize,
) -> Result<QTable, String> {
    read_columns_bounded(path, selected, 0, None, Some(filters), Some((max_cells, max_text)))
}

/// Read the projected SQL schema and validate filters using Parquet metadata.
pub fn parquet_dataset_columns(
    path: &Path,
    selected: &[String],
    filters: &[(&str, ParquetPredicate)],
) -> Result<Vec<QColumn>, String> {
    Ok(read_columns_bounded(path, selected, 0, None, Some(filters), None)?.columns)
}

fn read_columns_bounded(
    path: &Path,
    selected: &[String],
    rows_loaded: usize,
    expected: Option<&[QColumn]>,
    sql_filters: Option<&[(&str, ParquetPredicate)]>,
    limits: Option<(usize, usize)>,
) -> Result<QTable, String> {
    let reader = open(path)?;
    let meta = reader.metadata().file_metadata();
    let schema = meta.schema();
    let fields = schema.get_fields();
    let mut by_name = std::collections::HashMap::new();
    for field in fields {
        by_name.entry(field.name()).and_modify(|slot| *slot = None).or_insert(Some(field));
    }
    let mut projected = if selected.is_empty() {
        fields.to_vec()
    } else {
        selected.iter().map(|name| match by_name.get(name.as_str()) {
            Some(Some(field)) => Ok((*field).clone()),
            Some(None) => Err(format!("Ambiguous Parquet column: {name}")),
            None => Err(format!("Missing Parquet column: {name}")),
        }).collect::<Result<Vec<_>, String>>()?
    };
    let width = projected.len();
    let unique = projected.iter().map(|field| field.name()).collect::<std::collections::HashSet<_>>();
    if unique.len() != width {
        return Err("Duplicate Parquet column in projection".into());
    }
    let text_lists: Vec<_> = projected.iter().map(|field| sql_filters.is_some() && is_text_list(field)).collect();
    let columns = projected
        .iter()
        .zip(&text_lists)
        .map(|(field, &text_list)| {
            let name = field.name().to_string();
            let ty = column_type(field)
                .or_else(|| text_list.then_some(QType::Text))
                .ok_or_else(|| format!("Unsupported Parquet column: {name}"))?;
            Ok(QColumn { name, ty })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if expected.is_some_and(|expected| expected != columns) {
        return Err("Selected columns differ from the first Parquet file".into());
    }
    let filters = sql_filters.unwrap_or_default();
    let mut pruning_filters = Vec::new();
    let filter_indices = filters.iter().map(|(name, value)| {
        let field = match by_name.get(name) {
            Some(Some(field)) => *field,
            Some(None) => return Err(format!("Ambiguous Parquet column: {name}")),
            None => return Err(format!("Missing Parquet column: {name}")),
        };
        let ty = column_type(field).ok_or_else(|| format!("Unsupported Parquet column: {name}"))?;
        if !value.accepts_type(ty) {
            return Err(format!("Parquet filter has the wrong value type for column: {name}"));
        }
        let index = projected.iter().position(|field| field.name() == *name).unwrap_or_else(|| {
            projected.push(field.clone());
            projected.len() - 1
        });
        // A top-level primitive maps to exactly one physical leaf. Nested
        // columns can have the same leaf name, so compare the entire path.
        if let Some(leaf) = meta.schema_descr().columns().iter().position(|column| {
            column.path().parts() == [*name]
        }) {
            pruning_filters.push((leaf, meta.column_order(leaf), ty, value));
        }
        Ok((index, value))
    }).collect::<Result<Vec<_>, String>>()?;
    let Some((max_cells, mut max_text)) = limits else {
        return Ok(QTable { columns, rows: Vec::new() });
    };
    let rows = (rows_loaded as u64).saturating_add(meta.num_rows() as u64);
    if width > 16_384 || rows >= 1_048_576 || (filters.is_empty() && (width as u64).saturating_mul(rows) > max_cells as u64)
    {
        return Err("Parquet source exceeds the query editor's size limit".into());
    }
    let mut table = QTable {
        columns,
        rows: Vec::new(),
    };
    let projection = Type::group_type_builder(schema.name())
        .with_fields(projected)
        .build()
        .map_err(|e| e.to_string())?;
    for (index, metadata) in reader.metadata().row_groups().iter().enumerate() {
        if pruning_filters.iter().any(|&(leaf, order, ty, predicate)| {
            excludes_group(metadata, leaf, order, ty, predicate)
        }) {
            continue;
        }
        let group = reader.get_row_group(index).map_err(|e| e.to_string())?;
        'rows: for row in group
            .get_row_iter(Some(projection.clone()))
            .map_err(|e| e.to_string())?
        {
            let fields = row.map_err(|e| e.to_string())?.into_columns();
            for &(index, expected) in &filter_indices {
                let (name, field) = &fields[index];
                let (value, _) = cell(field.clone()).map_err(|e| format!("{name}: {e}"))?;
                if !expected.matches(&value) {
                    continue 'rows;
                }
            }
            if (rows_loaded + table.rows.len() + 1).saturating_mul(width) > max_cells {
                return Err("Parquet source exceeds the query editor's size limit".into());
            }
            let mut values = Vec::with_capacity(width);
            for ((name, field), text_list) in fields.into_iter().zip(text_lists.iter().copied()) {
                let value = if text_list {
                    text_list_cell(field, max_text)
                } else {
                    cell(field).map(|(value, _)| value)
                }.map_err(|e| format!("{name}: {e}"))?;
                if let CellValue::Text(text) = &value {
                    max_text = max_text
                        .checked_sub(text.len())
                        .ok_or("Parquet text exceeds the import limit")?;
                }
                values.push(value);
            }
            table.rows.push(values);
        }
    }
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_grouped_fixture(path: &Path, groups: i64, rows: i64, statistics: bool) {
        use parquet::data_type::{ByteArray, ByteArrayType, Int64Type};
        use parquet::file::{properties::{EnabledStatistics, WriterProperties}, writer::SerializedFileWriter};
        use parquet::schema::parser::parse_message_type;
        use std::sync::Arc;

        let schema = Arc::new(parse_message_type(
            "message points { REQUIRED INT64 id; OPTIONAL INT64 amount; REQUIRED BINARY label (UTF8); }",
        ).unwrap());
        let properties = WriterProperties::builder()
            .set_statistics_enabled(if statistics { EnabledStatistics::Chunk } else { EnabledStatistics::None })
            .set_compression(parquet::basic::Compression::SNAPPY)
            .build();
        let mut writer = SerializedFileWriter::new(File::create(path).unwrap(), schema, Arc::new(properties)).unwrap();
        for index in 0..groups {
            let mut group = writer.next_row_group().unwrap();
            let start = (index - groups / 2) * rows;
            let ids: Vec<_> = (start..start + rows).collect();
            let mut id = group.next_column().unwrap().unwrap();
            id.typed::<Int64Type>().write_batch(&ids, None, None).unwrap();
            id.close().unwrap();

            let defined = i16::from(index != 0);
            let amounts = if defined == 0 { &[][..] } else { ids.as_slice() };
            let mut amount = group.next_column().unwrap().unwrap();
            amount.typed::<Int64Type>().write_batch(amounts, Some(&vec![defined; ids.len()]), None).unwrap();
            amount.close().unwrap();

            let labels = vec![ByteArray::from(format!("{index:04}").as_str()); ids.len()];
            let mut label = group.next_column().unwrap().unwrap();
            label.typed::<ByteArrayType>().write_batch(&labels, None, None).unwrap();
            label.close().unwrap();
            group.close().unwrap();
        }
        writer.close().unwrap();
    }

    #[test]
    fn parquet_pruning_matches_a_scan_without_statistics() {
        use ParquetBound::{Ge, Lt};
        let dir = tempfile::tempdir().unwrap();
        let indexed = dir.path().join("indexed.parquet");
        let scan = dir.path().join("scan.parquet");
        write_grouped_fixture(&indexed, 6, 4, true);
        write_grouped_fixture(&scan, 6, 4, false);

        let range = ParquetPredicate::Range(vec![(Ge, CellValue::Number(-2.0)), (Lt, CellValue::Number(4.0))]);
        let text = ParquetPredicate::Range(vec![(Ge, CellValue::Text("0002".into())), (Lt, CellValue::Text("0004".into()))]);
        let missing = ParquetPredicate::Equal(CellValue::Empty);
        for (leaf, name, ty, predicate, expected) in [
            (1, "amount", QType::Whole, range, vec![-2, -1, 0, 1, 2, 3]),
            (2, "label", QType::Text, text, (-4..4).collect()),
            (1, "amount", QType::Whole, missing, (-12..-8).collect()),
        ] {
            let reader = open(&indexed).unwrap();
            let meta = reader.metadata();
            let excluded = meta.row_groups().iter().filter(|group| {
                excludes_group(group, leaf, meta.file_metadata().column_order(leaf), ty, &predicate)
            }).count();
            assert!(excluded >= 4, "expected pruning for {name}, got {excluded}");

            let filters = [(name, predicate)];
            let projected = ["id".into()];
            let actual = read_parquet_filtered(&indexed, &projected, &filters, 100, 100).unwrap();
            let full = read_parquet_filtered(&scan, &projected, &filters, 100, 100).unwrap();
            assert_eq!(actual.rows, full.rows);
            assert_eq!(actual.rows, expected.into_iter().map(|id| vec![CellValue::Number(id.into())]).collect::<Vec<_>>());
        }
    }

    #[test]
    #[ignore = "manual synthetic Parquet timing comparison"]
    fn benchmark_parquet_group_pruning() {
        use std::time::Instant;
        let dir = tempfile::tempdir().unwrap();
        let indexed = dir.path().join("indexed.parquet");
        let scan = dir.path().join("scan.parquet");
        write_grouped_fixture(&indexed, 128, 4096, true);
        write_grouped_fixture(&scan, 128, 4096, false);
        let predicate = ParquetPredicate::Range(vec![
            (ParquetBound::Ge, CellValue::Number(0.0)),
            (ParquetBound::Lt, CellValue::Number(4096.0)),
        ]);
        let filters = [("amount", predicate)];
        let projected = ["id".into(), "label".into()];
        let load = |path| read_parquet_filtered(path, &projected, &filters, 20_000, 100_000).unwrap();

        let expected = load(&scan);
        assert_eq!(expected.rows.len(), 4096);
        assert_eq!(load(&indexed).rows, expected.rows);
        let mut measurements = [Vec::new(), Vec::new()];
        let paths = [&scan, &indexed];
        for pair in 0..7 {
            for index in [pair % 2, 1 - pair % 2] {
                let start = Instant::now();
                let result = load(paths[index]);
                measurements[index].push(start.elapsed());
                assert_eq!(result.rows, expected.rows);
            }
        }
        for times in &mut measurements {
            times.sort_unstable();
        }
        eprintln!("524288 rows / 128 groups, 4096 rows matched, warm median of 7: scan {:?}, pruning {:?}",
            measurements[0][3], measurements[1][3]);
    }

    #[test]
    fn reads_a_snappy_table() {
        assert_reads_compressed_table(parquet::basic::Compression::SNAPPY, "snappy", false);
    }

    #[test]
    fn reads_a_zstandard_table() {
        assert_reads_compressed_table(
            parquet::basic::Compression::ZSTD(Default::default()),
            "zstd",
            false,
        );
    }

    #[test]
    fn projects_scalar_columns_around_nested_data() {
        assert_reads_compressed_table(parquet::basic::Compression::SNAPPY, "nested", true);
    }

    #[test]
    fn bounded_projection_counts_only_selected_scalar_columns() {
        let fixture = write_table(parquet::basic::Compression::SNAPPY, "bounded-projection", true);
        let selected = ["amount".into(), "code".into()];
        let table = read_parquet_bounded(&fixture, &selected, 4, 10).unwrap();
        assert_eq!(table.names(), ["amount", "code"]);
        assert_eq!(table.rows[1], [CellValue::Empty, CellValue::Text("00001".into())]);
        assert!(read_parquet_bounded(&fixture, &selected, 3, 10).is_err());
        assert!(read_parquet_bounded(&fixture, &selected, 4, 9).is_err());
        let error = read_parquet_bounded(&fixture, &["extra".into()], 4, 10).unwrap_err();
        assert_eq!(error, "Unsupported Parquet column: extra");
        std::fs::remove_file(fixture).unwrap();
    }

    #[test]
    fn parquet_filter_can_use_an_unreturned_column_before_the_cell_limit() {
        let fixture = write_table(parquet::basic::Compression::SNAPPY, "filtered-projection", true);
        let selected = ["code".into()];
        let table = read_parquet_filtered(&fixture, &selected, &[("amount", ParquetPredicate::Equal(CellValue::Empty))], 1, 5).unwrap();
        assert_eq!(table.names(), ["code"]);
        assert_eq!(table.rows, [vec![CellValue::Text("00001".into())]]);
        assert!(read_parquet_bounded(&fixture, &selected, 1, 5).is_err());
        assert!(read_parquet_filtered(&fixture, &selected, &[("amount", ParquetPredicate::Equal(CellValue::Text("7".into())))], 1, 5).is_err());
        std::fs::remove_file(fixture).unwrap();
    }

    #[test]
    fn parquet_ranges_apply_both_bounds_and_exclude_nulls() {
        use ParquetBound::{Ge, Gt, Le, Lt};
        let fixture = write_table(parquet::basic::Compression::SNAPPY, "range-projection", true);
        let selected = ["code".into()];
        let load = |lower, upper| read_parquet_filtered(
            &fixture,
            &selected,
            &[("amount", ParquetPredicate::Range(vec![
                (lower, CellValue::Number(7.0)),
                (upper, CellValue::Number(7.0)),
            ]))],
            1,
            5,
        ).unwrap();
        assert_eq!(load(Ge, Le).rows, [vec![CellValue::Text("01202".into())]]);
        assert!(load(Gt, Le).rows.is_empty());
        assert!(load(Ge, Lt).rows.is_empty());

        let text = read_parquet_filtered(&fixture, &selected, &[("code", ParquetPredicate::Range(vec![
            (Ge, CellValue::Text("00001".into())),
            (Lt, CellValue::Text("01202".into())),
        ]))], 1, 5).unwrap();
        assert_eq!(text.rows, [vec![CellValue::Text("00001".into())]]);
        std::fs::remove_file(fixture).unwrap();
    }

    #[test]
    fn queries_a_parquet_snapshot_with_nulls_and_scalar_types() {
        let fixture = write_table(parquet::basic::Compression::SNAPPY, "sql-dataset", false);
        let table = read_parquet_bounded(&fixture, &[], 100, 1024).unwrap();
        assert!(read_parquet_bounded(&fixture, &[], 1, 1024).is_err());
        let result = crate::query::sqlite_query_datasets(
            &[("source".into(), table)],
            "SELECT typeof(code), typeof(amount), count(*) FROM source GROUP BY 1,2 ORDER BY 2",
            &[],
            10,
            1024,
        )
        .unwrap();
        assert_eq!(
            result.rows,
            [
                vec![
                    serde_json::json!("text"),
                    serde_json::json!("integer"),
                    serde_json::json!(1)
                ],
                vec![
                    serde_json::json!("text"),
                    serde_json::json!("null"),
                    serde_json::json!(1)
                ],
            ]
        );
        std::fs::remove_file(fixture).unwrap();
    }

    fn write_table(
        compression: parquet::basic::Compression,
        fixture: &str,
        nested: bool,
    ) -> PathBuf {
        use parquet::data_type::{ByteArray, ByteArrayType, Int32Type, Int64Type};
        use parquet::file::{properties::WriterProperties, writer::SerializedFileWriter};
        use parquet::schema::parser::parse_message_type;
        use std::sync::Arc;

        let file = std::env::temp_dir().join(format!(
            "ibis-parquet-{fixture}-{}.parquet",
            std::process::id()
        ));
        let output = File::options()
            .write(true)
            .create_new(true)
            .open(&file)
            .unwrap();
        let extra = if nested {
            " OPTIONAL GROUP extra { REQUIRED INT32 item; }"
        } else {
            ""
        };
        let schema = Arc::new(
            parse_message_type(&format!(
                "message example {{ REQUIRED BINARY code (UTF8); OPTIONAL INT64 amount;{extra} }}"
            ))
            .unwrap(),
        );
        let properties = WriterProperties::builder()
            .set_compression(compression)
            .build();
        let mut writer = SerializedFileWriter::new(output, schema, Arc::new(properties)).unwrap();
        let mut group = writer.next_row_group().unwrap();
        let mut code = group.next_column().unwrap().unwrap();
        code.typed::<ByteArrayType>()
            .write_batch(
                &[ByteArray::from("01202"), ByteArray::from("00001")],
                None,
                None,
            )
            .unwrap();
        code.close().unwrap();
        let mut amount = group.next_column().unwrap().unwrap();
        amount
            .typed::<Int64Type>()
            .write_batch(&[7], Some(&[1, 0]), None)
            .unwrap();
        amount.close().unwrap();
        if nested {
            let mut extra = group.next_column().unwrap().unwrap();
            extra
                .typed::<Int32Type>()
                .write_batch(&[8], Some(&[1, 0]), None)
                .unwrap();
            extra.close().unwrap();
        }
        group.close().unwrap();
        writer.close().unwrap();

        file
    }

    #[test]
    fn imports_a_partitioned_folder_in_relative_path_order() {
        let directory = tempfile::tempdir().unwrap();
        let fixture = write_table(parquet::basic::Compression::SNAPPY, "folder-test", false);
        for month in ["02", "01"] {
            let path = directory
                .path()
                .join("year=2026")
                .join(format!("month={month}"));
            std::fs::create_dir_all(&path).unwrap();
            std::fs::copy(&fixture, path.join("part.parquet")).unwrap();
        }
        std::fs::remove_file(fixture).unwrap();
        let loaded = read_parquet_folder(directory.path(), &["code".into()]).unwrap();
        assert_eq!(loaded.names(), ["code", "month", "year"]);
        assert_eq!(loaded.rows.len(), 4);
        assert_eq!(loaded.rows[0][1], CellValue::Text("01".into()));
        assert_eq!(loaded.rows[2][1], CellValue::Text("02".into()));
    }

    #[test]
    fn appends_files_and_validates_projected_schema_and_limits() {
        let first = write_table(parquet::basic::Compression::SNAPPY, "part-01", false);
        let second = write_table(
            parquet::basic::Compression::ZSTD(Default::default()),
            "part-02",
            true,
        );
        let paths = [second.clone(), first.clone()];
        let schema = parquet_files_columns(&paths).unwrap();
        assert_eq!(
            schema.iter().map(|c| c.ty).collect::<Vec<_>>(),
            [Some(QType::Text), Some(QType::Whole), None]
        );
        let columns = vec!["amount".into(), "code".into()];
        let combined = read_parquet_files(&paths, &columns).unwrap();
        assert_eq!(combined.names(), ["amount", "code"]);
        assert_eq!(combined.rows.len(), 4);
        assert_eq!(
            combined.rows[0],
            [CellValue::Number(7.), CellValue::Text("01202".into())]
        );
        assert_eq!(combined.rows[..2], combined.rows[2..]);
        assert!(
            read_parquet_files(&[first.clone(), first.clone()], &columns)
                .unwrap_err()
                .contains("Duplicate")
        );
        assert!(
            read_columns(&first, &columns, 1_048_574, None)
                .unwrap_err()
                .contains("size limit")
        );
        let mut incompatible = combined.columns;
        incompatible[0].ty = QType::Text;
        assert!(
            read_columns(&second, &columns, 0, Some(&incompatible))
                .unwrap_err()
                .contains("differ")
        );
        std::fs::remove_file(first).unwrap();
        std::fs::remove_file(second).unwrap();
    }

    fn assert_reads_compressed_table(
        compression: parquet::basic::Compression,
        fixture: &str,
        nested: bool,
    ) {
        let file = write_table(compression, fixture, nested);
        let result = if nested {
            let schema = parquet_columns(&file).unwrap();
            assert_eq!(schema[0].ty, Some(QType::Text));
            assert_eq!(schema[1].ty, Some(QType::Whole));
            assert_eq!(schema[2].name, "extra");
            assert_eq!(schema[2].ty, None);
            assert!(read_parquet_columns(&file, &["extra".into()]).is_err());
            assert!(read_parquet_columns(&file, &["missing".into()]).is_err());
            let reordered = read_parquet_columns(&file, &["amount".into(), "code".into()]).unwrap();
            assert_eq!(reordered.names(), ["amount", "code"]);
            assert_eq!(
                reordered.rows[0],
                [CellValue::Number(7.), CellValue::Text("01202".into())]
            );
            read_parquet_columns(&file, &["code".into(), "amount".into()])
        } else {
            read_parquet(&file)
        };
        std::fs::remove_file(file).unwrap();
        let table = result.unwrap();
        assert_eq!(table.names(), ["code", "amount"]);
        assert_eq!(
            table.rows,
            [
                vec![CellValue::Text("01202".into()), CellValue::Number(7.)],
                vec![CellValue::Text("00001".into()), CellValue::Empty],
            ]
        );
    }

    #[test]
    fn preserves_codes_and_missing_values() {
        assert_eq!(
            cell(Field::Str("01202".into())).unwrap(),
            (CellValue::Text("01202".into()), QType::Text)
        );
        assert_eq!(cell(Field::Null).unwrap(), (CellValue::Empty, QType::Any));
        assert_eq!(
            cell(Field::Long(1202)).unwrap(),
            (CellValue::Number(1202.), QType::Whole)
        );
        assert_eq!(
            cell(Field::Date(0)).unwrap(),
            (CellValue::Number(25_569.), QType::Date)
        );
        assert!(cell(Field::Long(i64::MAX)).is_err());
    }
}
