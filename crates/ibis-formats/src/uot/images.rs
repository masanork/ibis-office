// SPDX-License-Identifier: AGPL-3.0-only

use std::collections::HashMap;
use std::path::Path;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use ibis_doc_model::{
    Emu, EmuRect, EmuSize, FloatingAnchor, FlowRunKind, MediaFormat, MediaId, Picture,
    Presentation, Shape, ShapeKind,
};
use roxmltree::Node;

use super::{UOF, WORD, word_child};

const GRAPH: &str = "http://schemas.uof.org/cn/2003/graph";
const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;
const TOTAL_IMAGE_BYTES: usize = 64 * 1024 * 1024;

pub(super) struct Images<'xml, 'input, 'path> {
    objects: HashMap<&'xml str, Node<'xml, 'input>>,
    graphics: HashMap<&'xml str, &'xml str>,
    media: HashMap<&'xml str, Option<MediaId>>,
    base_dir: Option<&'path Path>,
    emu_per_unit: f64,
    remaining: usize,
}

impl<'xml, 'input, 'path> Images<'xml, 'input, 'path> {
    pub(super) fn new(root: Node<'xml, 'input>, base_dir: Option<&'path Path>) -> Self {
        let object_set = root
            .children()
            .find(|node| node.has_tag_name((UOF, "对象集")));
        let objects = object_set
            .into_iter()
            .flat_map(|set| set.children())
            .filter(|node| node.has_tag_name((UOF, "其他对象")))
            .filter_map(|node| Some((node.attribute((UOF, "标识符"))?, node)))
            .collect();
        let graphics = object_set
            .into_iter()
            .flat_map(|set| set.children())
            .filter(|node| node.has_tag_name((GRAPH, "图形")))
            .filter_map(|node| {
                Some((
                    node.attribute((GRAPH, "标识符"))?,
                    node.attribute((GRAPH, "其他对象"))?,
                ))
            })
            .collect();
        let unit = root
            .children()
            .find(|node| node.has_tag_name((UOF, "文字处理")))
            .and_then(|node| word_child(node, "公用处理规则"))
            .and_then(|rules| word_child(rules, "文档设置"))
            .and_then(|settings| word_child(settings, "度量单位"))
            .and_then(|unit| unit.text());
        let emu_per_unit = match unit.map(str::trim) {
            Some("mm") => 36_000.0,
            Some("pt") => 12_700.0,
            Some("inch") => 914_400.0,
            Some("pica") => 152_400.0,
            _ => 360_000.0,
        };
        Self {
            objects,
            graphics,
            media: HashMap::new(),
            base_dir,
            emu_per_unit,
            remaining: TOTAL_IMAGE_BYTES,
        }
    }

    pub(super) fn picture(
        &mut self,
        anchor: Node<'_, '_>,
        presentation: &mut Presentation,
    ) -> Option<FlowRunKind> {
        let properties = word_child(anchor, "锚点属性")?;
        let dimension = |name| {
            self.length(word_child(properties, name)?.text()?)
                .filter(|&length| length > 0)
        };
        let size = EmuSize::new(dimension("宽度")?, dimension("高度")?);
        let reference = word_child(anchor, "图形")?.attribute((WORD, "图形引用"))?;
        // LibreOffice can store a frame name in 图:其他对象 while the actual
        // binary object uses the same identifier as the graphic.
        let (&id, &object) = self.objects.get_key_value(reference).or_else(|| {
            self.graphics
                .get(reference)
                .and_then(|id| self.objects.get_key_value(id))
        })?;
        let media = if let Some(media) = self.media.get(id) {
            *media
        } else {
            let media = self.read(object).map(|(format, bytes)| {
                self.remaining -= bytes.len();
                presentation.add_media(format, bytes)
            });
            self.media.insert(id, media);
            media
        }?;
        if anchor.attribute((WORD, "类型")) == Some("normal") {
            let floating = self.floating_anchor(properties);
            let mut shape = Shape::new(
                presentation.new_shape_id(),
                EmuRect::new(
                    floating.offset_x,
                    floating.offset_y,
                    size.width,
                    size.height,
                ),
                ShapeKind::Picture(Picture::new(media)),
            );
            shape.floating = Some(floating);
            return Some(FlowRunKind::Shape(Box::new(shape)));
        }
        Some(FlowRunKind::Picture {
            media,
            size,
            look: Default::default(),
        })
    }

    fn length(&self, raw: &str) -> Option<Emu> {
        let emu = (raw.trim().parse::<f64>().ok()? * self.emu_per_unit).round();
        (f64::from(i32::MIN)..=f64::from(i32::MAX))
            .contains(&emu)
            .then_some(emu as Emu)
    }

    fn floating_anchor(&self, properties: Node<'_, '_>) -> FloatingAnchor {
        let position = word_child(properties, "位置");
        let h = position.and_then(|node| word_child(node, "水平"));
        let v = position.and_then(|node| word_child(node, "垂直"));
        let coordinate = |axis: Option<Node<'_, '_>>, alignments: &[&str]| {
            let offset = axis
                .and_then(|node| word_child(node, "绝对"))
                .and_then(|node| node.attribute((WORD, "值")))
                .and_then(|raw| self.length(raw));
            let alignment = axis
                .filter(|_| offset.is_none())
                .and_then(|node| word_child(node, "相对"))
                .and_then(|node| node.attribute((WORD, "值")))
                .filter(|value| alignments.contains(value))
                .map(str::to_owned);
            (offset.unwrap_or(0), alignment)
        };
        let (offset_x, align_h) = coordinate(h, &["left", "center", "right", "inside", "outside"]);
        let (offset_y, align_v) = coordinate(v, &["top", "center", "bottom", "inside", "outside"]);
        let relative_h = match h.and_then(|node| node.attribute((WORD, "相对于"))) {
            Some("page") => "page",
            Some("margin") => "margin",
            Some("char") => "character",
            _ => "column",
        };
        let relative_v = match v.and_then(|node| node.attribute((WORD, "相对于"))) {
            Some("page") => "page",
            Some("margin") => "margin",
            Some("line") => "line",
            _ => "paragraph",
        };
        let wrapping = word_child(properties, "绕排");
        let mode = wrapping.and_then(|node| node.attribute((WORD, "绕排方式")));
        let wrap = match mode {
            Some("tight") => "tight",
            Some("through") => "through",
            Some("top-bottom" | "none") => "topAndBottom",
            Some("infrontoftext" | "behindtext") => "none",
            _ => "square",
        };
        let wrap_text = match wrap {
            "square" | "tight" | "through" => Some(
                match wrapping.and_then(|node| node.attribute((WORD, "环绕文字"))) {
                    Some("left") => "left",
                    Some("right") => "right",
                    Some("largest") => "largest",
                    _ => "bothSides",
                }
                .to_owned(),
            ),
            _ => None,
        };
        let margins = word_child(properties, "边距");
        let distance = |side| {
            margins
                .and_then(|node| node.attribute((WORD, side)))
                .and_then(|raw| self.length(raw))
                .unwrap_or(0)
                .max(0)
        };
        let flag = |name| match word_child(properties, name)
            .and_then(|node| node.attribute((WORD, "值")))
        {
            Some("true" | "1") => Some(true),
            Some("false" | "0") => Some(false),
            _ => None,
        };
        FloatingAnchor {
            offset_x,
            offset_y,
            align_h,
            align_v,
            relative_h: relative_h.to_owned(),
            relative_v: relative_v.to_owned(),
            wrap: wrap.to_owned(),
            wrap_text,
            behind_doc: mode == Some("behindtext"),
            locked: flag("锁定").unwrap_or(false),
            allow_overlap: flag("允许重叠").unwrap_or(true),
            dist_t: distance("上"),
            dist_b: distance("下"),
            dist_l: distance("左"),
            dist_r: distance("右"),
            ..Default::default()
        }
    }

    fn read(&self, object: Node<'_, '_>) -> Option<(MediaFormat, Vec<u8>)> {
        let limit = self.remaining.min(MAX_IMAGE_BYTES);
        let field = |name| {
            object
                .children()
                .find(|node| node.has_tag_name((UOF, name)))
        };
        let bytes = if let Some(data) = field("数据") {
            let encoded: Vec<_> = data
                .text()?
                .bytes()
                .filter(|byte| !byte.is_ascii_whitespace())
                .collect();
            if encoded.len() > limit.div_ceil(3) * 4 {
                return None;
            }
            STANDARD.decode(encoded).ok()?
        } else {
            relative_image(self.base_dir?, field("路径")?.text()?.trim(), limit)?
        };
        if bytes.len() > limit {
            return None;
        }
        let format = MediaFormat::sniff(&bytes)?;
        matches!(
            format,
            MediaFormat::Png | MediaFormat::Jpeg | MediaFormat::Gif | MediaFormat::Bmp
        )
        .then_some((format, bytes))
    }
}

#[cfg(not(target_family = "wasm"))]
fn relative_image(base: &Path, reference: &str, limit: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    use std::path::Component;

    let relative = Path::new(reference);
    if reference.is_empty()
        || reference.contains([':', '\\'])
        || relative.components().any(|part| {
            !matches!(
                part,
                Component::Normal(_) | Component::CurDir | Component::ParentDir
            )
        })
    {
        return None;
    }
    let base = if base.as_os_str().is_empty() {
        Path::new(".")
    } else {
        base
    };
    let base = base.canonicalize().ok()?;
    let target = base.join(relative).canonicalize().ok()?;
    if !target.starts_with(&base) || !target.is_file() {
        return None;
    }
    let file = std::fs::File::open(target).ok()?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() <= limit).then_some(bytes)
}

#[cfg(target_family = "wasm")]
fn relative_image(_base: &Path, _reference: &str, _limit: usize) -> Option<Vec<u8>> {
    None
}
