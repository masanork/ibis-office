// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::collections::HashMap;

use roxmltree::Node;

const DRAWING: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const VISIO_THEME: &str = "http://schemas.microsoft.com/office/visio/2012/theme";
pub(super) const REL: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme";

type Rgba = [u8; 4];

pub(super) struct Theme {
    pub(super) index: Option<u16>,
    palette: HashMap<&'static str, Rgba>,
    variants: Vec<[Option<Rgba>; 4]>,
    variation_fills: Vec<[Option<u16>; 4]>,
    fills: Vec<Option<Solid>>,
}

struct Solid {
    named_color: Option<String>,
    rgba: Option<Rgba>,
    shade: f32,
}

fn element<'a, 'i>(node: Node<'a, 'i>, ns: &str, name: &str) -> Option<Node<'a, 'i>> {
    node.children().find(|n| n.has_tag_name((ns, name)))
}

fn rgb(s: &str) -> Option<Rgba> {
    if s.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(s, 16).ok()?;
    Some([(n >> 16) as u8, (n >> 8) as u8, n as u8, 255])
}

fn node_color(n: Node<'_, '_>) -> Option<Rgba> {
    let attr = if n.has_tag_name((DRAWING, "srgbClr")) {
        "val"
    } else if n.has_tag_name((DRAWING, "sysClr")) {
        "lastClr"
    } else {
        return None;
    };
    rgb(n.attribute(attr)?)
}

impl Theme {
    pub(super) fn read(root: Node<'_, '_>) -> Option<Self> {
        let elements = element(root, DRAWING, "themeElements")?;
        let colors = element(elements, DRAWING, "clrScheme")?;
        let visio = |tag| {
            colors
                .descendants()
                .find(|n| n.has_tag_name((VISIO_THEME, tag)))
        };
        let mut palette: HashMap<_, _> = [
            "dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5",
            "accent6", "hlink", "folHlink",
        ]
        .into_iter()
        .filter_map(|key| {
            Some((
                key,
                node_color(element(colors, DRAWING, key)?.first_element_child()?)?,
            ))
        })
        .collect();
        if let Some(background) = visio("bkgnd").and_then(|n| node_color(n.first_element_child()?))
        {
            palette.insert("bg", background);
        }
        let variants = colors
            .descendants()
            .filter(|n| n.has_tag_name((VISIO_THEME, "variationClrScheme")))
            .take(16)
            .map(|n| {
                std::array::from_fn(|i| {
                    node_color(
                        element(
                            n,
                            VISIO_THEME,
                            ["varColor1", "varColor2", "varColor3", "varColor4"][i],
                        )?
                        .first_element_child()?,
                    )
                })
            })
            .collect();
        let format = element(elements, DRAWING, "fmtScheme");
        let variation_fills = format
            .into_iter()
            .flat_map(|n| n.descendants())
            .filter(|n| n.has_tag_name((VISIO_THEME, "variationStyleScheme")))
            .take(16)
            .map(|n| {
                std::array::from_fn(|i| {
                    n.children()
                        .filter(|n| n.has_tag_name((VISIO_THEME, "varStyle")))
                        .nth(i)?
                        .attribute("fillIdx")?
                        .parse()
                        .ok()
                })
            })
            .collect();
        let fills = format
            .and_then(|n| element(n, DRAWING, "fillStyleLst"))
            .into_iter()
            .flat_map(|n| n.children().filter(Node::is_element))
            .take(16)
            .map(|n| {
                if !n.has_tag_name((DRAWING, "solidFill")) {
                    return None;
                }
                let value = n.first_element_child()?;
                // Other DrawingML color transforms and gradient fills are outside the solid subset.
                if value
                    .children()
                    .filter(Node::is_element)
                    .any(|n| !n.has_tag_name((DRAWING, "shade")))
                {
                    return None;
                }
                let shade = element(value, DRAWING, "shade")
                    .and_then(|n| n.attribute("val")?.parse::<u32>().ok())
                    .unwrap_or(100_000)
                    .min(100_000) as f32
                    / 100_000.0;
                Some(Solid {
                    named_color: value
                        .has_tag_name((DRAWING, "schemeClr"))
                        .then(|| value.attribute("val").map(str::to_owned))
                        .flatten(),
                    rgba: node_color(value),
                    shade,
                })
            })
            .collect();
        Some(Self {
            index: visio("schemeID").and_then(|n| n.attribute("schemeEnum")?.parse().ok()),
            palette,
            variants,
            variation_fills,
            fills,
        })
    }

    pub(super) fn color(&self, index: u16, variant: u16) -> Option<Rgba> {
        match index {
            0..=8 => self
                .palette
                .get(
                    [
                        "dk1", "lt1", "accent1", "accent2", "accent3", "accent4", "accent5",
                        "accent6", "bg",
                    ][usize::from(index)],
                )
                .copied(),
            100..=103 => self.variants.get(usize::from(variant))?[usize::from(index - 100)],
            _ => None,
        }
    }

    pub(super) fn fill(&self, base: Option<Rgba>, index: u16, variant: u16) -> Option<Rgba> {
        let slot = match index {
            100..=103 => self.variation_fills.get(usize::from(variant))?[usize::from(index - 100)]?,
            value => value,
        };
        let fill = self
            .fills
            .get(usize::from(slot.checked_sub(1)?))?
            .as_ref()?;
        let mut color = match fill.named_color.as_deref() {
            Some("phClr") => base?,
            Some(name) => *self.palette.get(name)?,
            None => fill.rgba?,
        };
        for channel in &mut color[..3] {
            *channel = (f32::from(*channel) * fill.shade).round() as u8;
        }
        Some(color)
    }
}
