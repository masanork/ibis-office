// SPDX-License-Identifier: AGPL-3.0-only
//! Structured reader for Lotus Notes DXL (`.dxl`) and FileMaker DDR / `fmresultset` XML exports.

use crate::Error;
use roxmltree::{Document, Node};
use serde_json::{Value, json};

const MAX_XML_RECORDS: usize = 200;

fn child_text(node: Node<'_, '_>, tag: &str) -> Option<String> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == tag)
        .and_then(|n| n.text())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn collect_dxl_item_value(item: Node<'_, '_>) -> Value {
    if let Some(txt) = child_text(item, "text") {
        return json!(txt);
    }
    if let Some(num) = child_text(item, "number") {
        if let Ok(n) = num.parse::<f64>()
            && n.is_finite()
        {
            return json!(n);
        }
        return json!(num);
    }
    if let Some(dt) = child_text(item, "datetime") {
        return json!(dt);
    }
    let list_vals: Vec<Value> = item
        .descendants()
        .filter(|n| n.is_element() && matches!(n.tag_name().name(), "text" | "number" | "datetime"))
        .filter_map(|n| n.text().map(str::trim).filter(|s| !s.is_empty()).map(|s| json!(s)))
        .collect();
    if list_vals.is_empty() {
        Value::Null
    } else if list_vals.len() == 1 {
        list_vals.into_iter().next().unwrap()
    } else {
        Value::Array(list_vals)
    }
}

fn parse_lotus_dxl(doc: &Document<'_>) -> Value {
    let root = doc.root_element();
    let mut forms = Vec::new();
    let mut views = Vec::new();
    let mut agents = Vec::new();
    let mut documents = Vec::new();

    for node in root.descendants().filter(|n| n.is_element()) {
        match node.tag_name().name() {
            "form" => {
                let name = node.attribute("name").unwrap_or("").to_owned();
                let fields: Vec<_> = node
                    .descendants()
                    .filter(|n| n.is_element() && n.tag_name().name() == "field")
                    .map(|f| {
                        json!({
                            "name": f.attribute("name").unwrap_or(""),
                            "type": f.attribute("type").unwrap_or("text"),
                            "kind": f.attribute("kind").unwrap_or("editable"),
                        })
                    })
                    .collect();
                forms.push(json!({ "name": name, "fields": fields }));
            }
            "view" => {
                let name = node.attribute("name").unwrap_or("").to_owned();
                let columns: Vec<_> = node
                    .descendants()
                    .filter(|n| n.is_element() && n.tag_name().name() == "column")
                    .map(|c| {
                        let title = c
                            .children()
                            .find(|n| n.is_element() && n.tag_name().name() == "columnheader")
                            .and_then(|h| h.attribute("title"))
                            .unwrap_or("");
                        let formula = c
                            .descendants()
                            .find(|n| n.is_element() && n.tag_name().name() == "formula")
                            .and_then(|f| f.text())
                            .map(str::trim)
                            .unwrap_or("");
                        json!({
                            "itemname": c.attribute("itemname").unwrap_or(""),
                            "title": title,
                            "formula": formula,
                        })
                    })
                    .collect();
                views.push(json!({ "name": name, "columns": columns }));
            }
            "agent" => {
                agents.push(json!({
                    "name": node.attribute("name").unwrap_or(""),
                }));
            }
            "document" if documents.len() < MAX_XML_RECORDS => {
                let mut items = serde_json::Map::new();
                for item in node
                    .children()
                    .filter(|n| n.is_element() && n.tag_name().name() == "item")
                {
                    if let Some(name) = item.attribute("name") {
                        items.insert(name.to_owned(), collect_dxl_item_value(item));
                    }
                }
                documents.push(json!({
                    "form": node.attribute("form").unwrap_or(""),
                    "items": items,
                }));
            }
            _ => {}
        }
    }

    json!({
        "format": "lotus_dxl",
        "title": root.attribute("title").unwrap_or(""),
        "path": root.attribute("path").unwrap_or(""),
        "summary": {
            "forms": forms.len(),
            "views": views.len(),
            "agents": agents.len(),
            "documents": documents.len(),
        },
        "forms": forms,
        "views": views,
        "agents": agents,
        "documents": documents,
    })
}

fn parse_filemaker_ddr(doc: &Document<'_>) -> Value {
    let root = doc.root_element();
    let mut files = Vec::new();

    for file_node in root
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "File")
    {
        let mut tables = Vec::new();
        let mut relationships = Vec::new();
        let mut layouts = Vec::new();
        let mut scripts = Vec::new();

        for node in file_node.descendants().filter(|n| n.is_element()) {
            match node.tag_name().name() {
                "BaseTable" => {
                    let fields: Vec<_> = node
                        .descendants()
                        .filter(|n| n.is_element() && n.tag_name().name() == "Field")
                        .map(|f| {
                            let calc = child_text(f, "Calculation");
                            json!({
                                "name": f.attribute("name").unwrap_or(""),
                                "data_type": f.attribute("dataType").unwrap_or(""),
                                "field_type": f.attribute("fieldType").unwrap_or(""),
                                "calculation": calc,
                            })
                        })
                        .collect();
                    tables.push(json!({
                        "name": node.attribute("name").unwrap_or(""),
                        "records": node.attribute("records").and_then(|r| r.parse::<u64>().ok()),
                        "fields": fields,
                    }));
                }
                "Relationship" => {
                    let left = node
                        .children()
                        .find(|n| n.is_element() && n.tag_name().name() == "LeftTable")
                        .and_then(|n| n.attribute("name"))
                        .unwrap_or("");
                    let right = node
                        .children()
                        .find(|n| n.is_element() && n.tag_name().name() == "RightTable")
                        .and_then(|n| n.attribute("name"))
                        .unwrap_or("");
                    relationships.push(json!({
                        "left_table": left,
                        "right_table": right,
                    }));
                }
                "Layout" => {
                    layouts.push(json!({
                        "name": node.attribute("name").unwrap_or(""),
                    }));
                }
                "Script" => {
                    let step_count = node
                        .descendants()
                        .filter(|n| n.is_element() && n.tag_name().name() == "Step")
                        .count();
                    scripts.push(json!({
                        "name": node.attribute("name").unwrap_or(""),
                        "step_count": step_count,
                    }));
                }
                _ => {}
            }
        }

        files.push(json!({
            "name": file_node.attribute("name").unwrap_or(""),
            "path": file_node.attribute("path").unwrap_or(""),
            "tables": tables,
            "relationships": relationships,
            "layouts": layouts,
            "scripts": scripts,
        }));
    }

    json!({
        "format": "filemaker_ddr",
        "files": files,
    })
}

fn parse_filemaker_resultset(doc: &Document<'_>) -> Value {
    let root = doc.root_element();
    let datasource = root
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "datasource");
    let fields: Vec<_> = root
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "field")
        .filter(|n| n.parent().is_some_and(|p| p.tag_name().name() == "metadata"))
        .map(|f| {
            json!({
                "name": f.attribute("name").unwrap_or(""),
                "type": f.attribute("type").unwrap_or("TEXT"),
            })
        })
        .collect();

    let mut rows = Vec::new();
    for rec in root
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "record")
        .take(MAX_XML_RECORDS)
    {
        let row: Vec<String> = rec
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "col")
            .map(|col| child_text(col, "data").unwrap_or_default())
            .collect();
        rows.push(row);
    }

    json!({
        "format": "filemaker_fmresultset",
        "database": datasource.and_then(|d| d.attribute("database")).unwrap_or(""),
        "table": datasource.and_then(|d| d.attribute("table")).unwrap_or(""),
        "layout": datasource.and_then(|d| d.attribute("layout")).unwrap_or(""),
        "fields": fields,
        "rows": rows,
    })
}

/// Inspect a Lotus Notes DXL (`.dxl`) or FileMaker DDR / `fmresultset` XML export as structured JSON.
pub fn inspect_legacy_db_xml(xml_bytes: &[u8]) -> Result<Value, Error> {
    let text = std::str::from_utf8(xml_bytes)
        .map_err(|_| Error::new("Legacy XML export must be valid UTF-8"))?;
    let doc = Document::parse(text).map_err(|e| Error::new(format!("Invalid XML: {e}")))?;
    let root_name = doc.root_element().tag_name().name();
    match root_name {
        "database" | "document" => Ok(parse_lotus_dxl(&doc)),
        "FMPReport" => Ok(parse_filemaker_ddr(&doc)),
        "fmresultset" => Ok(parse_filemaker_resultset(&doc)),
        other => Err(Error::new(format!(
            "Unsupported legacy DB XML root element <{other}>; expected Lotus Notes DXL (<database>/<document>) or FileMaker (<FMPReport>/<fmresultset>)"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lotus_dxl_and_filemaker_ddr_xml() {
        let dxl = br#"<?xml version="1.0" encoding="UTF-8"?>
<database title="CustomerDB" path="crm.nsf">
  <form name="Customer">
    <field name="CompanyName" type="text" kind="editable"/>
  </form>
  <view name="AllCustomers">
    <column itemname="CompanyName"><columnheader title="Company"/><formula>CompanyName</formula></column>
  </view>
  <document form="Customer">
    <item name="CompanyName"><text>Acme Corp</text></item>
  </document>
</database>"#;
        let parsed_dxl = inspect_legacy_db_xml(dxl).unwrap();
        assert_eq!(parsed_dxl["format"], "lotus_dxl");
        assert_eq!(parsed_dxl["forms"][0]["name"], "Customer");
        assert_eq!(parsed_dxl["documents"][0]["items"]["CompanyName"], "Acme Corp");

        let ddr = br#"<?xml version="1.0" encoding="UTF-8"?>
<FMPReport type="Report">
  <File name="Orders.fmp12" path="file:Orders.fmp12">
    <BaseTableCatalog>
      <BaseTable name="Orders" records="42">
        <FieldCatalog><Field name="Total" dataType="Number" fieldType="Calculated"><Calculation>Qty * Price</Calculation></Field></FieldCatalog>
      </BaseTable>
    </BaseTableCatalog>
    <LayoutCatalog><Layout name="OrderEntry"/></LayoutCatalog>
    <ScriptCatalog><Script name="SubmitOrder"><Step name="Commit Records"/></Script></ScriptCatalog>
  </File>
</FMPReport>"#;
        let parsed_ddr = inspect_legacy_db_xml(ddr).unwrap();
        assert_eq!(parsed_ddr["format"], "filemaker_ddr");
        assert_eq!(parsed_ddr["files"][0]["tables"][0]["name"], "Orders");
        assert_eq!(
            parsed_ddr["files"][0]["tables"][0]["fields"][0]["calculation"],
            "Qty * Price"
        );
        assert_eq!(parsed_ddr["files"][0]["scripts"][0]["step_count"], 1);
    }
}
