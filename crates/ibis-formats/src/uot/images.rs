// SPDX-License-Identifier: AGPL-3.0-only

use std::collections::HashMap;
use std::path::Path;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use ibis_doc_model::{Emu, EmuSize, FlowRunKind, MediaFormat, MediaId, Presentation};
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
            let value = word_child(properties, name)?
                .text()?
                .trim()
                .parse::<f64>()
                .ok()?;
            let emu = (value * self.emu_per_unit).round();
            (1.0..=f64::from(i32::MAX))
                .contains(&emu)
                .then_some(emu as Emu)
        };
        let size = EmuSize::new(dimension("宽度")?, dimension("高度")?);
        let reference = word_child(anchor, "图形")?.attribute((WORD, "图形引用"))?;
        let id = self.graphics.get(reference).copied().unwrap_or(reference);
        let (&id, &object) = self.objects.get_key_value(id)?;
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
        Some(FlowRunKind::Picture {
            media,
            size,
            look: Default::default(),
        })
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
