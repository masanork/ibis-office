// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/229)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use super::*;
use super::syntax::parse_suppl_header;

    const SAMPLE_LAW_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Law Era="Heisei" Lang="ja" LawType="Act" Num="088" Year="05">
  <LawNum>平成五年法律第八十八号</LawNum>
  <LawBody>
    <LawTitle>行政手続法</LawTitle>
    <MainProvision>
      <Chapter Num="1">
        <ChapterTitle>第一章 総則</ChapterTitle>
        <Article Num="1">
          <ArticleCaption>（目的等）</ArticleCaption>
          <ArticleTitle>第一条</ArticleTitle>
          <Paragraph Num="1">
            <ParagraphNum></ParagraphNum>
            <ParagraphSentence><Sentence>この法律は、処分、行政指導及び届出に関する手続を定める。</Sentence></ParagraphSentence>
          </Paragraph>
          <Paragraph Num="2">
            <ParagraphNum>２</ParagraphNum>
            <ParagraphSentence><Sentence>他の法律に特別の定めがある場合は、その定めるところによる。</Sentence></ParagraphSentence>
            <Item Num="1">
              <ItemTitle>一</ItemTitle>
              <ItemSentence><Sentence>国の機関に対する処分</Sentence></ItemSentence>
            </Item>
          </Paragraph>
        </Article>
      </Chapter>
    </MainProvision>
    <SupplProvision>
      <SupplProvisionLabel>附則</SupplProvisionLabel>
      <Paragraph Num="1">
        <ParagraphNum></ParagraphNum>
        <ParagraphSentence><Sentence>この法律は、公布の日から起算して一年を超えない範囲内において施行する。</Sentence></ParagraphSentence>
      </Paragraph>
    </SupplProvision>
  </LawBody>
</Law>"#;

    #[test]
    fn law_xml_probe_handles_utf8_across_scan_boundary() {
        let markdown = format!("# Heading\n{}日本語", "a".repeat(4085));
        assert!(!is_law_xml(markdown.as_bytes()));
        assert!(input_to_markdown(markdown.as_bytes()).unwrap().is_none());
    }

    #[test]
    fn preserves_xml_paragraph_sentence_boundaries_and_attributes() {
        let original = SAMPLE_LAW_XML.replace(
            "<Sentence>この法律は、処分、行政指導及び届出に関する手続を定める。</Sentence>",
            concat!(
                "<Sentence Num=\"1\" Function=\"main\" WritingMode=\"horizontal\">本則を定める。</Sentence>",
                "<Sentence Num=\"2\" Function=\"proviso\">ただし、例外を設ける。</Sentence>",
            ),
        );
        let mut doc = parse_law_xml(&original).unwrap();
        assert!(to_law_xml(&doc).contains(concat!(
            "<Sentence Num=\"1\" Function=\"main\" WritingMode=\"horizontal\">本則を定める。</Sentence>",
            "<Sentence Num=\"2\" Function=\"proviso\">ただし、例外を設ける。</Sentence>",
        )));

        let lawtext = to_lawtext(&doc);
        let reparsed = parse_lawtext(&lawtext).unwrap();
        assert_eq!(to_law_xml(&reparsed), to_law_xml(&doc));

        let draft = generate_aratame_draft(&LawDoc::default(), &reparsed);
        assert!(draft.full_text.contains("本則を定める。ただし、例外を設ける。"));
        assert!(!draft.full_text.contains("<Sentence"));

        let LawNode::Division { children, .. } = &mut doc.main_provision[0] else {
            panic!("expected a chapter");
        };
        let LawNode::Article(article) = &mut children[0] else {
            panic!("expected an article");
        };
        let paragraph = &mut article.paragraphs[0];
        assert_eq!(paragraph.sentence, "本則を定める。ただし、例外を設ける。");
        paragraph.sentence = "本文を変更した。".into();
        let edited = to_law_xml(&doc);
        assert!(edited.contains("<ParagraphSentence><Sentence>本文を変更した。</Sentence>"));
        assert!(!edited.contains("Function=\"proviso\""));
    }

    #[test]
    fn roundtrips_law_xml_to_lawtext_and_back() {
        assert!(is_law_xml(SAMPLE_LAW_XML.as_bytes()));
        let parsed = parse_law_xml(SAMPLE_LAW_XML).expect("parse_law_xml");
        assert_eq!(parsed.law_title, "行政手続法");
        assert_eq!(parsed.law_num, "平成五年法律第八十八号");

        let lt = to_lawtext(&parsed);
        assert!(is_lawtext(lt.as_bytes()));
        assert!(lt.contains("行政手続法（平成五年法律第八十八号）"));
        assert!(lt.contains("（目的等）\n第一条 この法律は、処分、行政指導及び届出に関する手続を定める。"));
        assert!(lt.contains("２ 他の法律に特別の定めがある場合は、その定めるところによる。"));

        let reparsed = parse_lawtext(&lt).expect("parse_lawtext");
        assert_eq!(reparsed.law_title, parsed.law_title);
        assert_eq!(reparsed.law_num, parsed.law_num);
        assert_eq!(reparsed.main_provision, parsed.main_provision);

        let xml_out = to_law_xml(&reparsed);
        let xml_reparsed = parse_law_xml(&xml_out).expect("reparse generated xml");
        assert_eq!(xml_reparsed.main_provision, parsed.main_provision);
    }

    #[test]
    fn roundtrips_spaced_supplementary_provision_headings() {
        for label in ["附則", "附 則", "附　則"] {
            let xml = SAMPLE_LAW_XML
                .replace("<SupplProvision>", "<SupplProvision AmendLawNum=\"令和元年法律第一号\">")
                .replace("<SupplProvisionLabel>附則</SupplProvisionLabel>", &format!(
                    "<SupplProvisionLabel>{label}</SupplProvisionLabel>"
                ));
            let original = parse_law_xml(&xml).unwrap();
            let lawtext = to_lawtext(&original);
            let roundtrip = parse_law_xml(&to_law_xml(&parse_lawtext(&lawtext).unwrap())).unwrap();
            assert_eq!(roundtrip.main_provision, original.main_provision);
            assert_eq!(roundtrip.suppl_provisions, original.suppl_provisions);
        }
        assert!(parse_suppl_header("附　則の規定").is_none());
    }

    #[test]
    fn roundtrips_supplementary_extract_with_sparse_numbering() {
        let lawtext = "試験法（令和元年法律第一号）\n\n附　則（令和二年法律第二号）（抄）\n\n第二条\n１\n４ 経過措置を定める。\n  一から二十五まで 略\n  二十六及び二十七 略\n  二十八 その他政令で定めるもの\n";
        let doc = parse_lawtext(lawtext).unwrap();
        let suppl = &doc.suppl_provisions[0];
        assert!(suppl.extract);
        assert_eq!(suppl.amend_law_num.as_deref(), Some("令和二年法律第二号"));
        let [LawNode::Article(article)] = suppl.nodes.as_slice() else {
            panic!("expected one supplementary article");
        };
        assert_eq!(article.paragraphs[0].num, "1");
        assert!(article.paragraphs[0].sentence.is_empty());
        assert_eq!(article.paragraphs[1].num, "4");
        let numbers: Vec<_> = article.paragraphs[1].items.iter().map(|item| item.num.as_str()).collect();
        assert_eq!(numbers, ["1:25", "26:27", "28"]);
        let from_xml = parse_law_xml(&to_law_xml(&doc)).unwrap();
        let from_lawtext = parse_lawtext(&to_lawtext(&from_xml)).unwrap();
        assert_eq!(from_lawtext.suppl_provisions, doc.suppl_provisions);
    }

    #[test]
    fn roundtrips_law_xml_via_markdown() {
        let parsed = parse_law_xml(SAMPLE_LAW_XML).expect("parse_law_xml");
        let md = to_markdown(&parsed);
        assert!(md.contains("x-ibis-format: lawtext"));
        assert!(md.contains("# 行政手続法"));
        assert!(md.contains("### 第一条 （目的等）"));

        let from_md = parse_markdown_law(&md).expect("parse_markdown_law");
        assert_eq!(from_md.law_title, "行政手続法");
        assert_eq!(from_md.law_num, "平成五年法律第八十八号");
        assert_eq!(from_md.era.as_deref(), Some("Heisei"));
        assert_eq!(from_md.main_provision, parsed.main_provision);

        let xml = to_law_xml(&from_md);
        assert!(xml.contains("<Law Era=\"Heisei\" Lang=\"ja\" LawType=\"Act\" Num=\"088\" Year=\"05\">"));
        assert!(xml.contains("<Article Num=\"1\">"));
    }

    #[test]
    fn generates_shinkyu_markdown_and_opens_as_docx() {
        let old = parse_law_xml(SAMPLE_LAW_XML).expect("old");
        let mut new = old.clone();
        if let LawNode::Division { children, .. } = &mut new.main_provision[0] {
            if let LawNode::Article(art) = &mut children[0] {
                art.paragraphs[0].sentence =
                    "この法律は、処分、行政指導、届出及び命令等に関する手続を定める。".to_string();
            }
            children.push(LawNode::Article(LawArticle {
                num: "2".to_string(),
                caption: Some("（定義）".to_string()),
                title: "第二条".to_string(),
                paragraphs: vec![LawParagraph {
                    labels: LawParagraphLabels::default(),
                    num: "1".to_string(),
                    sentence: "この法律において用語の意義を定める。".to_string(),
                    xml_sentence_content: LawSentenceContent::default(),
                    items: Vec::new(),
                }],
            }));
        }

        let shinkyu_md = generate_shinkyu_markdown(&old, &new);
        assert!(shinkyu_md.contains("# 行政手続法 新旧対照表"));
        assert!(shinkyu_md.contains("| 改正後 | 現行 |"));
        assert!(shinkyu_md.contains("行政指導++、++届出++及び命令等++"), "unexpected shinkyu_md:\n{shinkyu_md}");
        assert!(shinkyu_md.contains("行政指導~~及び~~届出"), "unexpected shinkyu_md:\n{shinkyu_md}");
        assert!(shinkyu_md.contains("（新設）"));

        let docx = ibis_formats::markdown::open_markdown(shinkyu_md.as_bytes().to_vec())
            .expect("open shinkyu markdown as docx");
        assert!(docx.presentation.flow.is_some());
    }

    #[test]
    fn only_law_files_are_read_as_law() {
        use std::path::Path;
        // A CSV and a text file that look like Lawtext by their content (（）in the first line, a line of 第一条) are not laws.
        let looks = "名簿（令和七年）,a,b\n第一条,c,d\n".as_bytes();
        for name in ["list.csv", "memo.txt", "notes.md", "plain.xml", "x.docx"] {
            assert!(path_input_to_markdown(Path::new(name), looks).unwrap().is_none(), "{name}");
        }
        // A .xml that is not a law (Office 2003 XML) is left to the other readers, and a broken .xml of a non-law does not fail.
        assert!(path_input_to_markdown(Path::new("a.xml"), b"<?xml version=\"1.0\"?><Workbook/>").unwrap().is_none());
        assert!(path_input_to_markdown(Path::new("a.xml"), b"\xff\xfe not utf-8").unwrap().is_none());
        // A .lawtext and a law .xml are read.
        let lt = "行政手続法（平成五年法律第八十八号）\n\n第一条　この法律は、目的を定める。\n";
        assert!(path_input_to_markdown(Path::new("a.lawtext"), lt.as_bytes()).unwrap().is_some());
        assert!(path_input_to_markdown(Path::new("a.law.txt"), lt.as_bytes()).unwrap().is_some());
    }

    #[test]
    fn handles_real_egov_xml_ruby_enact_quote_and_appdx_table() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<Law Era="Reiwa" Lang="ja" LawType="Act" Num="001" Year="07">
  <LawNum>令和七年法律第一号</LawNum>
  <LawBody>
    <LawTitle>情報公開法</LawTitle>
    <EnactStatement>情報公開法をここに公布する。</EnactStatement>
    <TOC><TOCLabel>目次</TOCLabel></TOC>
    <MainProvision>
      <Article Num="1">
        <ArticleTitle>第一条</ArticleTitle>
        <Paragraph Num="1">
          <ParagraphNum/>
          <ParagraphSentence>
            <Sentence>この法律は、<Ruby>行政<Rt>ぎょうせい</Rt></Ruby>の<QuoteStruct>「情報公開」</QuoteStruct>を定める。</Sentence>
          </ParagraphSentence>
          <List>
            <ListSentence>
              <Column><Sentence>第一欄</Sentence></Column>
              <Column><Sentence>第二欄</Sentence></Column>
            </ListSentence>
          </List>
        </Paragraph>
      </Article>
    </MainProvision>
    <AppdxTable>
      <AppdxTableTitle>別表第一</AppdxTableTitle>
      <RelatedArticleNum>第一条関係</RelatedArticleNum>
      <TableStruct>
        <Table>
          <TableRow>
            <TableColumn><Sentence>区分</Sentence></TableColumn>
            <TableColumn><Sentence>手数料</Sentence></TableColumn>
          </TableRow>
          <TableRow>
            <TableColumn><Sentence>電子申請</Sentence></TableColumn>
            <TableColumn><Sentence>三百円</Sentence></TableColumn>
          </TableRow>
        </Table>
      </TableStruct>
    </AppdxTable>
  </LawBody>
</Law>"#;

        let parsed = parse_law_xml(xml).expect("parse real e-Gov XML");
        assert_eq!(parsed.enact_statement.as_deref(), Some("情報公開法をここに公布する。"));
        assert_eq!(parsed.appdx_tables.len(), 1);
        assert_eq!(parsed.appdx_tables[0].title, "別表第一");
        assert_eq!(parsed.appdx_tables[0].related_article_num.as_deref(), Some("第一条関係"));
        assert_eq!(parsed.appdx_tables[0].rows.len(), 2);

        if let LawNode::Article(a) = &parsed.main_provision[0] {
            assert_eq!(a.paragraphs[0].sentence, "この法律は、行政の「情報公開」を定める。");
        } else {
            panic!("expected Article");
        }

        let xml_roundtrip = to_law_xml(&parsed);
        let reparsed = parse_law_xml(&xml_roundtrip).expect("reparse XML");
        assert_eq!(reparsed.enact_statement, parsed.enact_statement);
        assert_eq!(reparsed.appdx_tables, parsed.appdx_tables);

        let md = to_markdown(&parsed);
        assert!(md.contains("## 別表第一（第一条関係）"));
        assert!(md.contains("| 区分 | 手数料 |"));
    }

    #[test]
    fn supports_arabic_numeral_corporate_and_municipal_rules() {
        let old_lt = "\
就業規則（2025年規程第1号）
第1条 社員の服務規律を定める。
第2条の2 特別休暇を定める。";
        let new_lt = "\
就業規則（2025年規程第1号）
第1条 社員及び嘱託社員の服務規律を定める。
第2条の2 特別休暇及び育児休暇を定める。";

        let old_doc = parse_lawtext(old_lt).unwrap();
        let new_doc = parse_lawtext(new_lt).unwrap();
        assert_eq!(old_doc.main_provision.len(), 2);

        let shinkyu = generate_shinkyu_markdown(&old_doc, &new_doc);
        assert!(shinkyu.contains("社員++及び嘱託社員++の服務規律"));
        assert!(shinkyu.contains("特別休暇++及び育児休暇++を定める"));
    }
