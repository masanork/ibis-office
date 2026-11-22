// SPDX-License-Identifier: AGPL-3.0-only
//! Read and update the text fields of an existing AcroForm.

use std::collections::{BTreeMap, BTreeSet};

use crate::{CosDict, CosObj, CosString, Error, ObjectId, PdfContainer, Result};

pub struct TextField {
    pub(crate) id: ObjectId,
    pub(crate) dict: CosDict,
    pub(crate) widgets: Vec<(ObjectId, CosDict)>,
    pub multiline: bool,
    pub max_len: Option<usize>,
    pub alignment: i64,
}

impl TextField {
    pub fn widget_count(&self) -> usize {
        self.widgets.len()
    }
}

pub fn text_fields(pdf: &mut PdfContainer) -> Result<BTreeMap<String, TextField>> {
    let (_, catalog) = pdf.catalog()?;
    let acro = dictionary(pdf, &catalog, b"AcroForm")?;
    if acro.get(b"XFA").is_some() {
        return Err(Error::new("XFA forms are unsupported"));
    }
    if matches!(acro.get(b"NeedAppearances"), Some(CosObj::Bool(true))) {
        return Err(Error::new("AcroForm requests viewer-generated appearances"));
    }
    let mut result = BTreeMap::new();
    let mut visited = BTreeSet::new();
    let defaults = Properties {
        alignment: acro.get_int(b"Q").unwrap_or(0),
        ..Properties::default()
    };
    let mut pending = array(pdf, &acro, b"Fields")?
        .into_iter()
        .map(|field| (field, String::new(), defaults.clone(), 0))
        .collect::<Vec<_>>();
    while let Some((field, parent_name, mut properties, depth)) = pending.pop() {
        let CosObj::Ref(id) = field else {
            return Err(Error::new("AcroForm field must be indirect"));
        };
        if depth > 64 || !visited.insert(id) {
            return Err(Error::new("Invalid AcroForm field tree"));
        }
        let CosObj::Dict(dict) = pdf.resolve(id)? else {
            return Err(Error::new("Invalid AcroForm field dictionary"));
        };
        let name = if let Some(CosObj::Str(name)) = dict.get(b"T") {
            let name = name.to_text_lossy();
            if parent_name.is_empty() {
                name
            } else {
                format!("{parent_name}.{name}")
            }
        } else {
            parent_name
        };
        if let Some(kind) = dict.get_name(b"FT") {
            properties.kind = kind.to_vec();
        }
        properties.flags = dict.get_int(b"Ff").unwrap_or(properties.flags);
        properties.alignment = dict.get_int(b"Q").unwrap_or(properties.alignment);
        properties.limit = dict.get_int(b"MaxLen").or(properties.limit);
        let mut widgets = Vec::new();
        if dict.get_name(b"Subtype") == Some(b"Widget") {
            widgets.push((id, dict.clone()));
        }
        let mut branches = false;
        for child in array(pdf, &dict, b"Kids")? {
            let CosObj::Ref(widget_id) = child else {
                return Err(Error::new("AcroForm child must be indirect"));
            };
            let CosObj::Dict(widget) = pdf.resolve(widget_id)? else {
                return Err(Error::new("Invalid AcroForm child dictionary"));
            };
            if widget.get_name(b"Subtype") == Some(b"Widget")
                && widget.get(b"T").is_none()
                && widget.get(b"Kids").is_none()
            {
                if widget
                    .get_name(b"FT")
                    .is_some_and(|kind| kind != properties.kind)
                {
                    return Err(Error::new("AcroForm widget has a conflicting field type"));
                }
                if !visited.insert(widget_id) {
                    return Err(Error::new("Repeated AcroForm widget reference"));
                }
                widgets.push((widget_id, widget));
            } else {
                branches = true;
                pending.push((child, name.clone(), properties.clone(), depth + 1));
            }
        }
        if branches && !widgets.is_empty() {
            return Err(Error::new("AcroForm field mixes widgets and child fields"));
        }
        if properties.kind == b"Sig" && dict.get(b"V").is_some_and(|value| *value != CosObj::Null) {
            return Err(Error::new("Signed PDFs cannot be filled"));
        }
        if widgets.is_empty() || properties.kind != b"Tx" {
            continue;
        }
        // Read-only, password, file-select, comb and rich-text fields need other semantics.
        if properties.flags & (1 | 1 << 13 | 1 << 20 | 1 << 24 | 1 << 25) != 0 {
            continue;
        }
        let max_len = properties
            .limit
            .map(usize::try_from)
            .transpose()
            .map_err(|_| Error::new("Invalid AcroForm /MaxLen"))?;
        if name.is_empty()
            || result
                .insert(
                    name,
                    TextField {
                        id,
                        dict,
                        widgets,
                        multiline: properties.flags & (1 << 12) != 0,
                        max_len,
                        alignment: properties.alignment,
                    },
                )
                .is_some()
        {
            return Err(Error::new("Missing or duplicate AcroForm field name"));
        }
    }
    Ok(result)
}

#[derive(Clone, Default)]
struct Properties {
    kind: Vec<u8>,
    flags: i64,
    limit: Option<i64>,
    alignment: i64,
}

fn dictionary(pdf: &mut PdfContainer, dict: &CosDict, key: &[u8]) -> Result<CosDict> {
    if let Some(value) = dict.get(key)
        && let CosObj::Dict(dict) = pdf.deref(value)?
    {
        return Ok(dict);
    }
    Err(Error::new(format!(
        "Missing or invalid /{}",
        String::from_utf8_lossy(key)
    )))
}

fn array(pdf: &mut PdfContainer, dict: &CosDict, key: &[u8]) -> Result<Vec<CosObj>> {
    match dict.get(key).map(|value| pdf.deref(value)).transpose()? {
        Some(CosObj::Array(values)) => Ok(values),
        None if key == b"Kids" => Ok(Vec::new()),
        _ => Err(Error::new(format!(
            "Missing or invalid /{}",
            String::from_utf8_lossy(key)
        ))),
    }
}

pub struct WidgetAppearance {
    pub width: f32,
    pub height: f32,
    pub background: Option<[u8; 3]>,
    pub border: Option<([u8; 3], f32)>,
}

fn widget_color(dict: &CosDict, key: &[u8]) -> Result<Option<[u8; 3]>> {
    let Some(value) = dict.get(key) else {
        return Ok(None);
    };
    let CosObj::Array(values) = value else {
        return Err(Error::new("Invalid widget color"));
    };
    let component = |value: &CosObj| {
        value
            .as_f64()
            .filter(|n| (0.0..=1.0).contains(n))
            .map(|n| (n * 255.0).round() as u8)
            .ok_or_else(|| Error::new("Invalid widget color component"))
    };
    match values.as_slice() {
        [] => Ok(None),
        [gray] => Ok(Some([component(gray)?; 3])),
        [r, g, b] => Ok(Some([component(r)?, component(g)?, component(b)?])),
        _ => Err(Error::new("Unsupported widget color space")),
    }
}

impl TextField {
    pub fn widget_appearances(&self, pdf: &mut PdfContainer) -> Result<Vec<WidgetAppearance>> {
        self.widgets
            .iter()
            .map(|(_, widget)| {
                let rect = widget
                    .get_array(b"Rect")
                    .ok_or_else(|| Error::new("Widget lacks /Rect"))?;
                let [x0, y0, x1, y1] = rect else {
                    return Err(Error::new("Invalid widget /Rect"));
                };
                let dimension = |first: &CosObj, last: &CosObj| {
                    let size = (last.as_f64()? - first.as_f64()?).abs() as f32;
                    (size.is_finite() && (5.0..=14400.0).contains(&size)).then_some(size)
                };
                let (Some(width), Some(height)) = (dimension(x0, x1), dimension(y0, y1)) else {
                    return Err(Error::new("Unsupported widget rectangle size"));
                };
                let mk = if widget.get(b"MK").is_some() {
                    dictionary(pdf, widget, b"MK")?
                } else {
                    CosDict::new()
                };
                if mk.get_int(b"R").unwrap_or(0).rem_euclid(360) != 0 || widget.get(b"AS").is_some()
                {
                    return Err(Error::new(
                        "Rotated widgets and appearance states are unsupported",
                    ));
                }
                let background = widget_color(&mk, b"BG")?;
                let border_color = widget_color(&mk, b"BC")?;
                let border = if let Some(color) = border_color {
                    let border_width = if widget.get(b"BS").is_some() {
                        let bs = dictionary(pdf, widget, b"BS")?;
                        if bs.get_name(b"S").is_some_and(|style| style != b"S") {
                            return Err(Error::new("Only solid widget borders are supported"));
                        }
                        bs.get_num(b"W").unwrap_or(1.0)
                    } else if let Some(values) = widget.get_array(b"Border") {
                        let [rx, ry, border_width] = values else {
                            return Err(Error::new(
                                "Only square, solid widget borders are supported",
                            ));
                        };
                        if rx.as_f64() != Some(0.0) || ry.as_f64() != Some(0.0) {
                            return Err(Error::new("Only square widget borders are supported"));
                        }
                        border_width
                            .as_f64()
                            .ok_or_else(|| Error::new("Invalid widget border width"))?
                    } else {
                        1.0
                    };
                    if !(0.0..f64::from(width.min(height) / 2.0)).contains(&border_width) {
                        return Err(Error::new("Invalid widget border width"));
                    }
                    (border_width > 0.0).then_some((color, border_width as f32))
                } else {
                    None
                };
                Ok(WidgetAppearance {
                    width,
                    height,
                    background,
                    border,
                })
            })
            .collect()
    }

    pub fn set_text(
        self,
        pdf: &mut PdfContainer,
        value: &str,
        appearances: Vec<ObjectId>,
    ) -> Result<()> {
        if appearances.len() != self.widgets.len() {
            return Err(Error::new("Expected one appearance per widget"));
        }
        let pdf_value = CosObj::Str(CosString::Hex(
            [0xfe, 0xff]
                .into_iter()
                .chain(value.encode_utf16().flat_map(u16::to_be_bytes))
                .collect(),
        ));
        let mut field = self.dict;
        field.set(b"V", pdf_value.clone());
        for ((id, mut widget), appearance) in self.widgets.into_iter().zip(appearances) {
            let mut ap = CosDict::new();
            ap.set(b"N", CosObj::Ref(appearance));
            if id == self.id {
                field.set(b"AP", CosObj::Dict(ap));
            } else {
                if widget.get(b"V").is_some() {
                    widget.set(b"V", pdf_value.clone());
                }
                widget.set(b"AP", CosObj::Dict(ap));
                pdf.set_object(id, CosObj::Dict(widget));
            }
        }
        pdf.set_object(self.id, CosObj::Dict(field));
        Ok(())
    }
}
