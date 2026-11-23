// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/229)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Statutory document and amendment-comparison model.

/// Structural division kind within `MainProvision` or `SupplProvision`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DivisionKind {
    Part,
    Chapter,
    Section,
    Subsection,
    Division,
}

impl DivisionKind {
    pub(super) fn xml_tag(self) -> &'static str {
        match self {
            Self::Part => "Part",
            Self::Chapter => "Chapter",
            Self::Section => "Section",
            Self::Subsection => "Subsection",
            Self::Division => "Division",
        }
    }

    pub(super) fn title_tag(self) -> &'static str {
        match self {
            Self::Part => "PartTitle",
            Self::Chapter => "ChapterTitle",
            Self::Section => "SectionTitle",
            Self::Subsection => "SubsectionTitle",
            Self::Division => "DivisionTitle",
        }
    }

    pub(super) fn from_xml_tag(tag: &str) -> Option<Self> {
        match tag {
            "Part" => Some(Self::Part),
            "Chapter" => Some(Self::Chapter),
            "Section" => Some(Self::Section),
            "Subsection" => Some(Self::Subsection),
            "Division" => Some(Self::Division),
            _ => None,
        }
    }

    pub(super) fn heading_level(self) -> usize {
        match self {
            Self::Part => 2,
            Self::Chapter => 2,
            Self::Section => 3,
            Self::Subsection => 4,
            Self::Division => 5,
        }
    }
}

/// A sub-item (`Subitem1`..`Subitem10`) inside an [`LawItem`].
#[derive(Debug, Clone)]
pub struct LawSubitem {
    pub depth: u8,
    pub num: String,
    pub title: String,
    pub sentence: String,
    pub xml_sentence_content: LawSentenceContent,
    pub children: Vec<LawSubitem>,
}

impl PartialEq for LawSubitem {
    fn eq(&self, other: &Self) -> bool {
        self.depth == other.depth && self.num == other.num && self.title == other.title
            && self.sentence == other.sentence && self.children == other.children
    }
}

impl Eq for LawSubitem {}

/// An item (`Item`: 一, 二, 三...) inside a [`LawParagraph`].
#[derive(Debug, Clone)]
pub struct LawItem {
    pub num: String,
    pub title: String,
    pub sentence: String,
    pub xml_sentence_content: LawSentenceContent,
    pub subitems: Vec<LawSubitem>,
}

impl PartialEq for LawItem {
    fn eq(&self, other: &Self) -> bool {
        self.num == other.num && self.title == other.title
            && self.sentence == other.sentence && self.subitems == other.subitems
    }
}

impl Eq for LawItem {}

/// A sentence as recorded inside a law XML paragraph, item or sub-item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LawSentence {
    pub text: String,
    pub num: Option<String>,
    pub function: Option<String>,
    pub indent: Option<String>,
    pub writing_mode: Option<String>,
    // Escaped XML containing text and validated Ruby/Rt elements only.
    pub(super) ruby_markup: Option<String>,
}

/// Column as used in an XML item or sub-item sentence container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LawSentenceColumn {
    pub num: Option<String>,
    pub line_break: Option<String>,
    pub align: Option<String>,
    pub sentences: Vec<LawSentence>,
}

/// The XML structure behind the flat text used for amendments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LawSentenceContent {
    Sentences(Vec<LawSentence>),
    Columns(Vec<LawSentenceColumn>),
}

impl Default for LawSentenceContent {
    fn default() -> Self {
        Self::Sentences(Vec::new())
    }
}

impl LawSentenceContent {
    pub(super) fn plain_text(&self) -> Option<String> {
        let sentence_text = |sentences: &[LawSentence]| -> String {
            sentences.iter().map(|sentence| sentence.text.as_str()).collect()
        };
        match self {
            Self::Sentences(sentences) if !sentences.is_empty() => Some(sentence_text(sentences)),
            Self::Columns(columns) if !columns.is_empty() => Some(columns.iter()
                .map(|column| sentence_text(&column.sentences))
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>().join(" ")),
            _ => None,
        }
    }

    pub(super) fn has_structure(&self) -> bool {
        match self {
            Self::Columns(_) => true,
            Self::Sentences(sentences) => sentences.len() > 1 || sentences.iter().any(|sentence| {
                sentence.num.is_some() || sentence.function.is_some()
                    || sentence.indent.is_some() || sentence.writing_mode.is_some()
                    || sentence.ruby_markup.is_some()
            }),
        }
    }
}

/// Display labels carried by a paragraph independently of its identifier.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LawParagraphLabels {
    pub caption: Option<String>,
    pub common_caption: Option<String>,
    pub number: Option<String>,
}

/// A paragraph (`Paragraph`: 項) inside a [`LawArticle`] or [`SupplProvision`].
#[derive(Debug, Clone)]
pub struct LawParagraph {
    pub num: String,
    pub labels: LawParagraphLabels,
    pub sentence: String,
    /// Original XML sentence boundaries and attributes, used when the text still matches.
    pub xml_sentence_content: LawSentenceContent,
    pub items: Vec<LawItem>,
}

// Amendment comparisons operate on the text. XML serialization details do not
// make a paragraph differ from an equivalent plain-text paragraph.
impl PartialEq for LawParagraph {
    fn eq(&self, other: &Self) -> bool {
        self.num == other.num && self.sentence == other.sentence && self.items == other.items
    }
}

impl Eq for LawParagraph {}

/// An article (`Article`: 条).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LawArticle {
    pub num: String,
    pub caption: Option<String>,
    pub title: String,
    pub paragraphs: Vec<LawParagraph>,
}

/// A node inside `MainProvision` or `SupplProvision`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LawNode {
    Division {
        kind: DivisionKind,
        num: String,
        title: String,
        children: Vec<LawNode>,
    },
    Article(LawArticle),
    Paragraph(LawParagraph),
}

/// A supplementary provision (`SupplProvision`: 附則).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupplProvision {
    pub amend_law_num: Option<String>,
    pub extract: bool,
    pub label: String,
    pub nodes: Vec<LawNode>,
}

/// An appended table (`AppdxTable`: 別表).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LawAppdxTable {
    pub title: String,
    pub related_article_num: Option<String>,
    pub rows: Vec<Vec<String>>,
}

/// Reading and abbreviation attributes from the XML law title.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LawTitleAttributes {
    pub kana: Option<String>,
    pub abbrev: Option<String>,
    pub abbrev_kana: Option<String>,
}

/// Top-level AST for an e-Gov 法令標準XML / Lawtext document.
///
/// Equality of paragraphs, items and sub-items compares their provision text and
/// hierarchy. XML-only labels and sentence markup are checked by the XML audit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LawDoc {
    pub law_title: String,
    pub law_title_attributes: LawTitleAttributes,
    pub law_num: String,
    pub era: Option<String>,
    pub year: Option<String>,
    pub promulgate_month: Option<String>,
    pub promulgate_day: Option<String>,
    pub num: Option<String>,
    pub law_type: Option<String>,
    pub lang: Option<String>,
    pub enact_statement: Option<String>,
    pub(super) toc_xml: Option<String>,
    // Number of preceding tables for each style, when the appendices are interleaved.
    pub(super) appdx_style_positions: Vec<usize>,
    pub(super) appdx_style_xml: Vec<String>,
    pub preamble: Option<String>,
    pub main_provision: Vec<LawNode>,
    pub suppl_provisions: Vec<SupplProvision>,
    pub appdx_tables: Vec<LawAppdxTable>,
    pub(super) appdx_table_xml: Vec<super::table_markup::SavedAppdxTable>,
}

impl Default for LawDoc {
    fn default() -> Self {
        Self {
            law_title: String::new(),
            law_title_attributes: LawTitleAttributes::default(),
            law_num: String::new(),
            era: None,
            year: None,
            promulgate_month: None,
            promulgate_day: None,
            num: None,
            law_type: None,
            lang: Some("ja".to_string()),
            enact_statement: None,
            toc_xml: None,
            appdx_style_positions: Vec::new(),
            appdx_style_xml: Vec::new(),
            preamble: None,
            main_provision: Vec::new(),
            suppl_provisions: Vec::new(),
            appdx_tables: Vec::new(),
            appdx_table_xml: Vec::new(),
        }
    }
}
