// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! 5点セット（新旧対照表・改め文・参照条文・要綱・理由）の統合オーケストレーションおよび
//! ibisOffice 内 AI Assistant 連携用コンテキスト・プロンプト生成モジュール。

use serde_json::{Value, json};

use super::aratame::{AratameDraft, VerifyResult, generate_aratame_draft, verify_aratame};
use super::diff::{AmendmentIr, ShinkyuOptions, diff_laws, generate_shinkyu_markdown_with_options};
use super::lint::{LintReport, lint_amendment};
use super::refs::{LawReference, extract_references, generate_sansho_markdown};
use super::LawDoc;

/// Deterministic outputs + AI Assistant prompt package for the 5-piece legislative set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Legislation5SetPackage {
    pub ir: AmendmentIr,
    pub shinkyu_markdown: String,
    pub aratame_draft: AratameDraft,
    pub aratame_verification: VerifyResult,
    pub sansho_markdown: String,
    pub references: Vec<LawReference>,
    pub lint_report: LintReport,
    pub youkou_prompt: String,
    pub riyuu_prompt: String,
    pub aratame_polish_prompt: String,
}

impl Legislation5SetPackage {
    /// Serialize the entire 5-piece package into JSON for the in-app AI Assistant
    /// or Cloudflare Worker backend.
    pub fn to_json_value(&self) -> Value {
        json!({
            "law_title": self.ir.law_title,
            "amendment_ir": self.ir.to_json_value(),
            "deterministic_outputs": {
                "shinkyu_markdown": self.shinkyu_markdown,
                "aratame_draft": self.aratame_draft.to_json_value(),
                "aratame_verification": self.aratame_verification.to_json_value(),
                "sansho_markdown": self.sansho_markdown,
                "references": self.references.iter().map(LawReference::to_json_value).collect::<Vec<_>>(),
                "lint_report": self.lint_report.to_json_value(),
            },
            "assistant_prompts": {
                "youkou": self.youkou_prompt,
                "riyuu": self.riyuu_prompt,
                "aratame_polish": self.aratame_polish_prompt,
            }
        })
    }
}

/// Build a [`Legislation5SetPackage`] from `old` and `new` [`LawDoc`]s plus an optional
/// policy background memo (`policy_memo`) provided by the user in the AI Assistant.
pub fn build_5set_package(
    old: &LawDoc,
    new: &LawDoc,
    policy_memo: &str,
    shinkyu_opts: &ShinkyuOptions,
) -> Legislation5SetPackage {
    let ir = diff_laws(old, new);
    let shinkyu_markdown = generate_shinkyu_markdown_with_options(old, new, shinkyu_opts);
    let aratame_draft = generate_aratame_draft(old, new);
    let aratame_verification = verify_aratame(old, &aratame_draft.full_text, new);
    let references = extract_references(new);
    let sansho_markdown = generate_sansho_markdown(old, new);
    let lint_report = lint_amendment(old, new);

    let ir_summary = ir.to_json_string().unwrap_or_default();
    let memo_section = if policy_memo.trim().is_empty() {
        "（担当者メモなし：構造化差分から改正趣旨を推定してください）".to_string()
    } else {
        policy_memo.trim().to_string()
    };

    let youkou_prompt = format!(
        "あなたは日本の法制執務を支援するアシスタントです。\n\
         以下の法令改正の構造化差分（AmendmentIr）と担当者メモに基づき、「{}の一部を改正する法律案要綱」のドラフトをMarkdownで作成してください。\n\
         体裁は日本の標準的な要綱文体（「第一 目的」「第二 〇〇に関する事項」「一 〜すること。」の体言止め）に従ってください。\n\n\
         ## 担当者メモ\n{}\n\n## 構造化差分 (AmendmentIr)\n```json\n{}\n```",
        ir.law_title, memo_section, ir_summary
    );

    let riyuu_prompt = format!(
        "あなたは日本の法制執務を支援するアシスタントです。\n\
         以下の構造化差分と担当者メモに基づき、法律案の「理由」（提案理由）を起案してください。\n\
         末尾は標準的な「〜する必要がある。これが、この法律案を提出する理由である。」の形式で結んでください。\n\n\
         ## 担当者メモ\n{}\n\n## 構造化差分 (AmendmentIr)\n```json\n{}\n```",
        memo_section, ir_summary
    );

    let aratame_polish_prompt = format!(
        "あなたは日本の法制執務を支援するアシスタントです。\n\
         以下の自動生成された改め文ドラフトについて、法制執務上の表現（連用形接続や文言の自然さ）を整え、\
         必要に応じて附則（施行期日・経過措置）のたたき台を補ってください。\n\
         ただし、本則の改め文は機械検算（逆適用検証）に通るよう、引用語句「〜」を勝手に変更しないでください。\n\n\
         ## 現在の検算結果\n- exact_match: {}\n- diagnostics: {:?}\n\n\
         ## 改め文ドラフト\n```\n{}\n```",
        aratame_verification.is_exact_match,
        aratame_verification.diagnostics,
        aratame_draft.full_text
    );

    Legislation5SetPackage {
        ir,
        shinkyu_markdown,
        aratame_draft,
        aratame_verification,
        sansho_markdown,
        references,
        lint_report,
        youkou_prompt,
        riyuu_prompt,
        aratame_polish_prompt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lawtext::parse_lawtext;

    #[test]
    fn builds_complete_5set_package() {
        let old_lt = "\
テスト法（令和七年法律第一号）
（目的）
第一条 この法律は、処分に関する手続を定める。";
        let new_lt = "\
テスト法（令和七年法律第一号）
（目的）
第一条 この法律は、処分及び行政指導に関する手続を定める。";

        let old_doc = parse_lawtext(old_lt).unwrap();
        let new_doc = parse_lawtext(new_lt).unwrap();
        let pkg = build_5set_package(
            &old_doc,
            &new_doc,
            "行政指導の透明性向上のため手続対象に追加する。",
            &ShinkyuOptions::default(),
        );

        assert!(pkg.aratame_verification.is_exact_match);
        assert!(pkg.shinkyu_markdown.contains("新旧対照表"));
        assert!(pkg.youkou_prompt.contains("行政指導の透明性向上"));
        assert!(pkg.riyuu_prompt.contains("これが、この法律案を提出する理由である"));
        let json = pkg.to_json_value();
        assert_eq!(
            json["deterministic_outputs"]["aratame_verification"]["is_exact_match"],
            true
        );
    }
}
