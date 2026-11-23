// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/229)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Statutory document and amendment-comparison model.

use std::collections::BTreeMap;

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

impl SupplProvision {
    fn has_same_heading(&self, other: &Self) -> bool {
        self.amend_law_num == other.amend_law_num
            && self.extract == other.extract
            && self.label == other.label
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SupplAppendices {
    provision: SupplProvision,
    unique_heading: bool,
    occurrence: usize,
    fragments: Vec<String>,
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
    // Number of preceding tables for each non-table appendix, when the appendices are interleaved.
    pub(super) appdx_fragment_positions: Vec<usize>,
    pub(super) appdx_fragment_xml: Vec<String>,
    pub preamble: Option<String>,
    pub main_provision: Vec<LawNode>,
    // Number of top-level appendices preceding each supplementary provision.
    pub(super) suppl_positions: Vec<usize>,
    pub suppl_provisions: Vec<SupplProvision>,
    pub(super) suppl_appendices: BTreeMap<usize, SupplAppendices>,
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
            appdx_fragment_positions: Vec::new(),
            appdx_fragment_xml: Vec::new(),
            preamble: None,
            main_provision: Vec::new(),
            suppl_positions: Vec::new(),
            suppl_provisions: Vec::new(),
            suppl_appendices: BTreeMap::new(),
            appdx_tables: Vec::new(),
            appdx_table_xml: Vec::new(),
        }
    }
}

impl LawDoc {
    /// Edit the supplementary provision at its current index and retain its appendices.
    ///
    /// Prefer this method to editing the public vector directly when provisions
    /// share a heading. The index is resolved before the body or heading changes.
    /// Returns `None` if the index is out of bounds.
    pub fn edit_suppl_provision<R>(
        &mut self,
        index: usize,
        edit: impl FnOnce(&mut SupplProvision) -> R,
    ) -> Option<R> {
        let mut edited = self.suppl_provisions.get(index)?.clone();
        let result = edit(&mut edited);

        // Resolve current owners before changing the values used to recognize them.
        let owners: Vec<_> = (0..self.suppl_provisions.len())
            .filter_map(|index| self.suppl_appendix_key(index).map(|key| (index, key)))
            .collect();
        let mut previous = std::mem::take(&mut self.suppl_appendices);
        self.suppl_appendices = owners.into_iter()
            .filter_map(|(index, key)| previous.remove(&key).map(|saved| (index, saved)))
            .collect();
        self.suppl_provisions[index] = edited;
        self.bind_suppl_appendices();
        Some(result)
    }

    pub(super) fn push_suppl_provision(&mut self, suppl: SupplProvision, fragments: Vec<String>) {
        if !fragments.is_empty() {
            self.suppl_appendices.insert(self.suppl_provisions.len(), SupplAppendices {
                provision: suppl.clone(),
                unique_heading: false,
                occurrence: 0,
                fragments,
            });
        }
        self.suppl_provisions.push(suppl);
    }

    // Bind after parsing, when the complete bodies are available to distinguish repeated headings.
    pub(super) fn bind_suppl_appendices(&mut self) {
        for (&index, saved) in &mut self.suppl_appendices {
            let provision = &self.suppl_provisions[index];
            if saved.provision != *provision {
                saved.provision.clone_from(provision);
            }
            saved.unique_heading = self.suppl_provisions.iter()
                .filter(|other| provision.has_same_heading(other))
                .take(2)
                .count() == 1;
            saved.occurrence = self.suppl_provisions[..index].iter()
                .filter(|other| *other == provision)
                .count();
        }
    }

    fn suppl_appendix_key(&self, index: usize) -> Option<usize> {
        if self.suppl_appendices.is_empty() {
            return None;
        }
        let provision = self.suppl_provisions.get(index)?;
        let candidates = self.suppl_appendices.iter()
            .filter(|(_, saved)| provision.has_same_heading(&saved.provision));
        let occurrence = self.suppl_provisions[..index].iter()
            .filter(|other| *other == provision)
            .count();
        for (&key, saved) in candidates {
            if saved.provision == *provision && saved.occurrence == occurrence {
                return Some(key);
            }
            if saved.unique_heading
                && self.suppl_provisions.iter()
                    .filter(|other| provision.has_same_heading(other))
                    .take(2)
                    .count() == 1
            {
                return Some(key);
            }
        }
        None
    }

    pub(super) fn suppl_appendices(&self, index: usize) -> &[String] {
        self.suppl_appendix_key(index)
            .map_or(&[], |key| self.suppl_appendices[&key].fragments.as_slice())
    }

    pub(super) fn take_suppl_provisions_from(&mut self, source: &mut Self) {
        self.suppl_provisions = std::mem::take(&mut source.suppl_provisions);
        self.suppl_appendices = std::mem::take(&mut source.suppl_appendices);
    }

    pub(super) fn remove_suppl_provision(&mut self, index: usize) {
        if let Some(key) = self.suppl_appendix_key(index) {
            self.suppl_appendices.remove(&key);
        }
        self.suppl_provisions.remove(index);
        if index < self.suppl_positions.len() {
            self.suppl_positions.remove(index);
        }
    }
}
