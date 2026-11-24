// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::collections::BTreeMap;

#[cfg(not(target_family = "wasm"))]
use gpui_kit::PathPromptOptions;
use gpui_kit::{Context, Window};
use ibis_doc_model::Rgb;
use serde_json::Value;

use crate::{editor::SlideEditor, i18n};

const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_SWATCHES: usize = 512;

#[derive(Debug)]
struct Import {
    swatches: Vec<(String, Rgb)>,
    skipped: usize,
}

fn collect<'a>(
    node: &'a Value,
    path: &str,
    inherited_type: Option<&str>,
    colors: &mut BTreeMap<String, &'a Value>,
) {
    let Some(group) = node.as_object() else {
        return;
    };
    let kind = group
        .get("$type")
        .and_then(Value::as_str)
        .or(inherited_type);
    if let Some(value) = group.get("$value") {
        if kind == Some("color") && !path.is_empty() {
            colors.insert(path.to_owned(), value);
        }
        return;
    }
    for (name, value) in group {
        let child_path = match name.as_str() {
            "$root" => path.to_owned(),
            name if name.starts_with('$') => continue,
            name if path.is_empty() => name.to_owned(),
            name => format!("{path}.{name}"),
        };
        collect(value, &child_path, kind, colors);
    }
}

fn hex_rgb(value: &str) -> Option<Rgb> {
    let hex = value.strip_prefix('#')?;
    if !hex.bytes().all(|digit| digit.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        3 | 6 => {}
        4 if hex[3..].eq_ignore_ascii_case("f") => return hex_rgb(&value[..4]),
        8 if hex[6..].eq_ignore_ascii_case("ff") => return hex_rgb(&value[..7]),
        _ => return None,
    }
    let rgb = u32::from_str_radix(hex, 16).ok()?;
    Some(Rgb(if hex.len() == 3 {
        ((rgb >> 8) * 17 << 16) | (((rgb >> 4) & 15) * 17 << 8) | ((rgb & 15) * 17)
    } else {
        rgb
    }))
}

fn literal_rgb(value: &Value) -> Option<Rgb> {
    if let Some(text) = value.as_str() {
        return hex_rgb(text);
    }
    if value.get("colorSpace")?.as_str()? != "srgb"
        || value
            .get("alpha")
            .is_some_and(|alpha| alpha.as_f64() != Some(1.0))
    {
        return None;
    }
    let [red, green, blue] = value.get("components")?.as_array()?.as_slice() else {
        return None;
    };
    [red, green, blue]
        .into_iter()
        .try_fold(0, |rgb, component| {
            let level = component.as_f64()?;
            (0.0..=1.0)
                .contains(&level)
                .then(|| (rgb << 8) | (level * 255.0).round() as u32)
        })
        .map(Rgb)
}

fn resolve(
    value: &Value,
    colors: &BTreeMap<String, &Value>,
    set_aliases: &BTreeMap<&str, Option<&Value>>,
) -> Option<Rgb> {
    let mut value = value;
    // Bound alias traversal, including cycles.
    for _ in 0..64 {
        let alias = value
            .as_str()
            .and_then(|text| text.strip_prefix('{'))
            .and_then(|text| text.strip_suffix('}'));
        match alias {
            Some(path) => {
                value = colors
                    .get(path)
                    .copied()
                    .or_else(|| set_aliases.get(path).copied().flatten())?;
            }
            None => return literal_rgb(value),
        }
    }
    None
}

fn parse(bytes: &[u8]) -> Option<Import> {
    if bytes.len() > MAX_BYTES {
        return None;
    }
    let root: Value =
        serde_json::from_slice(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes)).ok()?;
    let mut colors = BTreeMap::new();
    collect(&root, "", None, &mut colors);
    // Tokens Studio wraps each set in an extra group. References omit the set name.
    let sets = root
        .pointer("/$metadata/tokenSetOrder")
        .and_then(Value::as_array);
    let mut set_aliases = BTreeMap::new();
    if let Some(sets) = sets {
        let set_names: std::collections::BTreeSet<_> =
            sets.iter().filter_map(Value::as_str).collect();
        for (path, &value) in &colors {
            if let Some((set, token)) = path.split_once('.')
                && set_names.contains(set)
            {
                set_aliases
                    .entry(token)
                    .and_modify(|candidate| *candidate = None)
                    .or_insert(Some(value));
            }
        }
    }
    let swatches: Vec<_> = colors
        .iter()
        .filter_map(|(name, value)| Some((name.clone(), resolve(value, &colors, &set_aliases)?)))
        .take(MAX_SWATCHES)
        .collect();
    (!swatches.is_empty()).then(|| Import {
        skipped: colors.len() - swatches.len(),
        swatches,
    })
}

impl SlideEditor {
    pub(crate) fn choose_design_tokens(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        #[cfg(not(target_family = "wasm"))]
        {
            let picked = cx.prompt_for_paths(PathPromptOptions {
                files: true,
                directories: false,
                extensions: vec!["json".into()],
                multiple: false,
                prompt: None,
            });
            cx.spawn_in(w, async move |this, cx| {
                use std::io::Read as _;
                let Ok(Ok(Some(paths))) = picked.await else {
                    return;
                };
                let Some(path) = paths.into_iter().next() else {
                    return;
                };
                let mut bytes = Vec::new();
                let result = std::fs::File::open(path)
                    .and_then(|file| file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes))
                    .ok()
                    .and_then(|_| parse(&bytes));
                this.update_in(cx, |this, _, cx| this.apply_design_tokens(result, cx))
                    .ok();
            })
            .detach();
        }
        #[cfg(target_family = "wasm")]
        cx.spawn_in(w, async move |this, cx| {
            let result = crate::web::pick_design_tokens(MAX_BYTES as u32).await;
            this.update_in(cx, |this, _, cx| match result {
                Ok(Some((_, bytes))) => this.apply_design_tokens(parse(&bytes), cx),
                Ok(None) => {}
                Err(_) => this.apply_design_tokens(None, cx),
            })
            .ok();
        })
        .detach();
    }

    fn apply_design_tokens(&mut self, imported: Option<Import>, cx: &mut Context<Self>) {
        let status = if let Some(imported) = imported {
            let count = imported.swatches.len();
            let skipped = imported.skipped;
            self.design_token_colors = imported.swatches;
            if skipped == 0 {
                i18n::format("color.tokens.loaded", &[("count", &count)])
            } else {
                i18n::format(
                    "color.tokens.partial",
                    &[("count", &count), ("skipped", &skipped)],
                )
            }
        } else {
            i18n::text("color.tokens.error").to_string()
        };
        self.set_status(status, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn design_tokens_resolve_named_opaque_colors() {
        let imported = parse(
            br##"{
            "colors": {
                "$type": "color",
                "base": {"$root": {"$value": "#abc"}, "dark": {"$value":"#112233ff"}},
                "alias": {"$value":"{colors.base}"},
                "structured": {"$value":{"colorSpace":"srgb","components":[0,0.5,1]}},
                "cycle": {"$value":"{colors.cycle}"},
                "missing": {"$value":"{colors.unknown}"},
                "alpha": {"$value":"rgba(0,0,0,0.1)"},
                "size": {"$type":"dimension", "$value":"10px"}
            }
        }"##,
        )
        .unwrap();
        let colors = imported.swatches.into_iter().collect::<BTreeMap<_, _>>();
        assert_eq!(colors.len(), 4);
        assert_eq!(colors["colors.base"], Rgb(0xaabbcc));
        assert_eq!(colors["colors.alias"], colors["colors.base"]);
        assert_eq!(colors["colors.base.dark"], Rgb(0x112233));
        assert_eq!(colors["colors.structured"], Rgb(0x0080ff));
        assert_eq!(imported.skipped, 3);
        assert!(parse(b"{}").is_none());
    }

    #[::core::prelude::v1::test]
    fn design_tokens_read_official_digital_agency_json() {
        let Some(path) = std::env::var_os("IBIS_DADS_TOKENS") else {
            return;
        };
        let imported = parse(&std::fs::read(path).unwrap()).unwrap();
        let colors = imported.swatches.into_iter().collect::<BTreeMap<_, _>>();
        assert_eq!(colors.len(), 165);
        assert_eq!(imported.skipped, 12);
        assert_eq!(
            colors["Color.Color.Key.500"],
            colors["Color.Color.Primitive.Blue.500"]
        );
    }
}
