// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Document outline extraction for the Right Dock (headings for Markdown/DOCX/JTD,
//! sheets for XLSX/CSV, slides/pages for PPTX/PDF).

use ibis_doc_model::flow_command::{all_paragraphs, heading_level};
use ibis_doc_model::{FlowRunKind, Presentation, ShapeKind, Slide, SlideId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutlineTarget {
    Paragraph(u32),
    Slide(SlideId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineItem {
    /// 1-based heading/outline depth (`1..=6`).
    pub level: usize,
    pub label: String,
    pub target: OutlineTarget,
}

fn slide_preview_title(slide: &Slide) -> Option<String> {
    if let Some(name) = slide.name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        return Some(name.to_owned());
    }
    for shape in &slide.shapes {
        let body = match &shape.kind {
            ShapeKind::TextBox(body) => Some(body),
            ShapeKind::Auto(auto) => auto.text.as_ref(),
            _ => None,
        };
        if let Some(body) = body {
            for para in &body.paragraphs {
                let text: String = para
                    .runs
                    .iter()
                    .map(|r| r.text.as_str())
                    .collect::<String>()
                    .trim()
                    .to_owned();
                if !text.is_empty() {
                    return Some(text.chars().take(60).collect());
                }
            }
        }
    }
    None
}

/// Extract a structured outline for any document hosted in [`SlideEditor`].
pub fn extract_outline(pres: &Presentation) -> Vec<OutlineItem> {
    if let Some(flow) = pres.flow.as_deref() {
        let mut items = Vec::new();
        for para in all_paragraphs(flow) {
            let Some(level) = heading_level(&flow.styles, para) else {
                continue;
            };
            let text: String = para
                .runs
                .iter()
                .filter_map(|run| match &run.kind {
                    FlowRunKind::Text(s) => Some(s.as_str()),
                    _ => None,
                })
                .collect::<String>()
                .trim()
                .to_owned();
            if text.is_empty() {
                continue;
            }
            items.push(OutlineItem {
                level: (level as usize).clamp(1, 6),
                label: text,
                target: OutlineTarget::Paragraph(para.id),
            });
        }
        return items;
    }

    if pres.workbook.is_some() {
        return pres
            .slides
            .iter()
            .enumerate()
            .map(|(idx, slide)| OutlineItem {
                level: 1,
                label: slide
                    .name
                    .clone()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| format!("Sheet {}", idx + 1)),
                target: OutlineTarget::Slide(slide.id),
            })
            .collect();
    }

    pres.slides
        .iter()
        .enumerate()
        .map(|(idx, slide)| {
            let label = match slide_preview_title(slide) {
                Some(title) => format!("{}. {}", idx + 1, title),
                None => format!("ページ {}", idx + 1),
            };
            OutlineItem {
                level: 1,
                label,
                target: OutlineTarget::Slide(slide.id),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use ibis_doc_model::{Block, FlowDocument, FlowParagraph, FlowRun, FlowStyle, Slide, SlideId};

    #[test]
    fn extracts_markdown_and_flow_headings_by_level() {
        let mut flow = FlowDocument::default();
        let styles = Arc::make_mut(&mut flow.styles);
        styles.styles.insert(
            "Heading1".into(),
            FlowStyle {
                name: "heading 1".into(),
                ..Default::default()
            },
        );
        styles.styles.insert(
            "Heading2".into(),
            FlowStyle {
                name: "heading 2".into(),
                ..Default::default()
            },
        );
        flow.body.push(Block::Paragraph(FlowParagraph {
            id: 10,
            style: Some("Heading1".into()),
            runs: vec![FlowRun::new(FlowRunKind::Text("Overview".into()), Default::default())],
            ..Default::default()
        }));
        flow.body.push(Block::Paragraph(FlowParagraph {
            id: 11,
            runs: vec![FlowRun::new(
                FlowRunKind::Text("Regular body paragraph".into()),
                Default::default(),
            )],
            ..Default::default()
        }));
        flow.body.push(Block::Paragraph(FlowParagraph {
            id: 12,
            style: Some("Heading2".into()),
            runs: vec![FlowRun::new(FlowRunKind::Text("Architecture".into()), Default::default())],
            ..Default::default()
        }));

        let mut pres = Presentation::new();
        pres.flow = Some(Arc::new(flow));

        let items = extract_outline(&pres);
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0],
            OutlineItem {
                level: 1,
                label: "Overview".into(),
                target: OutlineTarget::Paragraph(10)
            }
        );
        assert_eq!(
            items[1],
            OutlineItem {
                level: 2,
                label: "Architecture".into(),
                target: OutlineTarget::Paragraph(12)
            }
        );
    }

    #[test]
    fn extracts_slides_as_fallback_outline() {
        let mut pres = Presentation::new();
        pres.slides = vec![
            Slide {
                id: SlideId(1),
                name: Some("Intro".into()),
                ..Slide::new(SlideId(1))
            },
            Slide::new(SlideId(2)),
        ];

        let items = extract_outline(&pres);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].label, "1. Intro");
        assert_eq!(items[0].target, OutlineTarget::Slide(SlideId(1)));
        assert_eq!(items[1].label, "ページ 2");
        assert_eq!(items[1].target, OutlineTarget::Slide(SlideId(2)));
    }
}
