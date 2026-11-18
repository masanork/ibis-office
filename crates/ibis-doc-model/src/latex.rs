// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! LaTeX mathematical formula parser and converter (PPTX-MATH-012).
//!
//! Parses LaTeX math syntax (e.g. `\frac{a}{b}`, `\sqrt{x+1}`, `\sum_{i=1}^n x`,
//! `\alpha`, `\beta`, `\begin{matrix} ... \end{matrix}`) into a [`Math`] tree,
//! and converts [`Math`] trees back to standard LaTeX math syntax.

use crate::math::{FracKind, Math, stacked_by_default};

/// Parse a LaTeX math string into a [`Math`] tree.
pub fn parse_latex(text: &str) -> Math {
    let mut cursor = LatexParser {
        chars: text.chars().collect(),
        at: 0,
    };
    let items = cursor.parse_row(&[]);
    row_of(merge_text(items))
}

struct LatexParser {
    chars: Vec<char>,
    at: usize,
}

impl LatexParser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn peek_non_ws(&mut self) -> Option<char> {
        self.skip_ws();
        self.peek()
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.at += 1;
            } else {
                break;
            }
        }
    }

    fn parse_row(&mut self, stop: &[char]) -> Vec<Math> {
        let mut items = Vec::new();
        while let Some(c) = self.peek_non_ws() {
            if stop.contains(&c) {
                break;
            }
            if c == '&' || c == '\\' && self.peek_at(self.at + 1) == Some('\\') {
                // Table/matrix/align cell or row breaks handled in environment parsers
                break;
            }

            let atom = self.parse_atom();
            if let Some(atom) = atom {
                // Check if followed by subscript `_` or superscript `^`
                let with_scripts = self.attach_scripts(atom);
                items.push(with_scripts);
            }
        }
        items
    }

    fn peek_at(&self, idx: usize) -> Option<char> {
        self.chars.get(idx).copied()
    }

    fn parse_atom(&mut self) -> Option<Math> {
        self.skip_ws();
        let c = self.peek()?;
        match c {
            '{' => {
                self.at += 1;
                let inner = self.parse_row(&['}']);
                if self.peek() == Some('}') {
                    self.at += 1;
                }
                Some(clean_row(inner))
            }
            '\\' => {
                self.at += 1;
                Some(self.parse_command())
            }
            '(' => {
                self.at += 1;
                let inner = self.parse_row(&[')']);
                if self.peek() == Some(')') {
                    self.at += 1;
                }
                Some(Math::Delimited {
                    open: "(".into(),
                    close: ")".into(),
                    sep: "|".into(),
                    items: vec![clean_row(inner)],
                })
            }
            '[' => {
                self.at += 1;
                let inner = self.parse_row(&[']']);
                if self.peek() == Some(']') {
                    self.at += 1;
                }
                Some(Math::Delimited {
                    open: "[".into(),
                    close: "]".into(),
                    sep: "|".into(),
                    items: vec![clean_row(inner)],
                })
            }
            _ => {
                self.at += 1;
                Some(Math::Text(c.to_string()))
            }
        }
    }

    fn parse_command(&mut self) -> Math {
        let start = self.at;
        if let Some(c) = self.peek() {
            if !c.is_ascii_alphabetic() {
                self.at += 1;
                return match c {
                    '{' => Math::Text("{".into()),
                    '}' => Math::Text("}".into()),
                    '\\' => Math::Text("\n".into()),
                    ',' | ';' | ':' | ' ' | '!' => Math::Text(String::new()),
                    _ => Math::Text(c.to_string()),
                };
            }
        }
        while let Some(c) = self.peek() {
            if c.is_ascii_alphabetic() {
                self.at += 1;
            } else {
                break;
            }
        }
        let cmd: String = self.chars[start..self.at].iter().collect();
        match cmd.as_str() {
            // Fractions
            "frac" | "dfrac" | "tfrac" => {
                let num = self.parse_required_arg();
                let den = self.parse_required_arg();
                Math::Frac {
                    num: Box::new(num),
                    den: Box::new(den),
                    kind: FracKind::Bar,
                }
            }
            "sfrac" | "nicefrac" => {
                let num = self.parse_required_arg();
                let den = self.parse_required_arg();
                Math::Frac {
                    num: Box::new(num),
                    den: Box::new(den),
                    kind: FracKind::Skew,
                }
            }
            "lfrac" => {
                let num = self.parse_required_arg();
                let den = self.parse_required_arg();
                Math::Frac {
                    num: Box::new(num),
                    den: Box::new(den),
                    kind: FracKind::Linear,
                }
            }
            "nobarfrac" => {
                let num = self.parse_required_arg();
                let den = self.parse_required_arg();
                Math::Frac {
                    num: Box::new(num),
                    den: Box::new(den),
                    kind: FracKind::NoBar,
                }
            }
            "binom" => {
                let num = self.parse_required_arg();
                let den = self.parse_required_arg();
                Math::Delimited {
                    open: "(".into(),
                    close: ")".into(),
                    sep: String::new(),
                    items: vec![Math::Frac {
                        num: Box::new(num),
                        den: Box::new(den),
                        kind: FracKind::NoBar,
                    }],
                }
            }
            // Radicals
            "sqrt" => {
                self.skip_ws();
                let degree = if self.peek() == Some('[') {
                    self.at += 1;
                    let deg = self.parse_row(&[']']);
                    if self.peek() == Some(']') {
                        self.at += 1;
                    }
                    Some(Box::new(clean_row(deg)))
                } else {
                    None
                };
                let body = self.parse_required_arg();
                Math::Radical {
                    degree,
                    body: Box::new(body),
                }
            }
            // Big operators (N-ary)
            "sum" => self.parse_nary("∑"),
            "prod" => self.parse_nary("∏"),
            "coprod" => self.parse_nary("∐"),
            "int" => self.parse_nary("∫"),
            "iint" => self.parse_nary("∬"),
            "iiint" => self.parse_nary("∭"),
            "oint" => self.parse_nary("∮"),
            "bigcup" => self.parse_nary("⋃"),
            "bigcap" => self.parse_nary("⋂"),
            "bigvee" => self.parse_nary("⋁"),
            "bigwedge" => self.parse_nary("⋀"),

            // Delimiters: \left ... \right
            "left" => {
                self.skip_ws();
                let open = self.parse_delim_char();
                let inner = self.parse_row_until_right();
                let close = self.parse_delim_char();
                let sep = if open == "{" || open == "(" || open == "[" { "|".to_string() } else { String::new() };
                Math::Delimited {
                    open,
                    close,
                    sep,
                    items: vec![clean_row(inner)],
                }
            }
            "right" => {
                // Encountered without left
                let close = self.parse_delim_char();
                Math::Text(close)
            }

            // Environments: \begin{...} ... \end{...}
            "begin" => {
                let env = self.parse_required_arg_text();
                self.parse_environment(&env)
            }

            // Functions
            "sin" | "cos" | "tan" | "cot" | "sec" | "csc" | "arcsin" | "arccos" | "arctan"
            | "sinh" | "cosh" | "tanh" | "log" | "ln" | "exp" | "lim" | "min" | "max" | "det" => {
                let arg = if let Some('{') = self.peek_non_ws() {
                    self.parse_required_arg()
                } else {
                    Math::Row(Vec::new())
                };
                Math::Function {
                    name: Box::new(Math::Text(cmd)),
                    arg: Box::new(arg),
                }
            }

            // Accents
            "hat" => Math::Mark { base: Box::new(self.parse_required_arg()), mark: "^".into(), over: true },
            "vec" => Math::Mark { base: Box::new(self.parse_required_arg()), mark: "→".into(), over: true },
            "bar" | "overline" => Math::Mark { base: Box::new(self.parse_required_arg()), mark: "¯".into(), over: true },
            "dot" => Math::Mark { base: Box::new(self.parse_required_arg()), mark: "˙".into(), over: true },
            "ddot" => Math::Mark { base: Box::new(self.parse_required_arg()), mark: "¨".into(), over: true },
            "tilde" => Math::Mark { base: Box::new(self.parse_required_arg()), mark: "~".into(), over: true },
            "underline" => Math::Mark { base: Box::new(self.parse_required_arg()), mark: "_".into(), over: false },

            // Text modes
            "text" | "mathrm" | "mathbf" | "mathit" | "mathsf" | "mathtt" => {
                let text = self.parse_required_arg_text();
                Math::Text(text)
            }

            // Spacing
            "quad" | "qquad" => Math::Text(" ".into()),

            // Greek letters and math symbols
            _ => {
                if let Some(symbol) = lookup_latex_symbol(&cmd) {
                    Math::Text(symbol.into())
                } else {
                    Math::Text(format!(r"\{cmd}"))
                }
            }
        }
    }

    fn parse_nary(&mut self, op: &str) -> Math {
        let mut stacked = stacked_by_default(op);
        let mut sub = None;
        let mut sup = None;
        while let Some(c) = self.peek_non_ws() {
            if c == '\\' {
                let command: String = self.chars[self.at + 1..].iter().copied()
                    .take_while(char::is_ascii_alphabetic).collect();
                stacked = match command.as_str() {
                    "limits" => true,
                    "nolimits" => false,
                    _ => break,
                };
                self.at += command.len() + 1;
            } else if c == '_' {
                self.at += 1;
                sub = Some(Box::new(self.parse_required_arg()));
            } else if c == '^' {
                self.at += 1;
                sup = Some(Box::new(self.parse_required_arg()));
            } else {
                break;
            }
        }
        Math::Nary {
            op: op.to_string(),
            sub,
            sup,
            body: Box::new(Math::Row(Vec::new())),
            stacked,
        }
    }

    fn parse_delim_char(&mut self) -> String {
        self.skip_ws();
        let Some(c) = self.peek() else { return String::new() };
        self.at += 1;
        if c == '\\' {
            if let Some(next) = self.peek() {
                self.at += 1;
                match next {
                    '{' => "{".to_string(),
                    '}' => "}".to_string(),
                    '|' => "‖".to_string(),
                    _ => next.to_string(),
                }
            } else {
                "\\".to_string()
            }
        } else if c == '.' {
            String::new()
        } else {
            c.to_string()
        }
    }

    fn parse_row_until_right(&mut self) -> Vec<Math> {
        let mut items = Vec::new();
        while let Some(c) = self.peek_non_ws() {
            if c == '\\' {
                let at_mark = self.at;
                self.at += 1;
                let start = self.at;
                while let Some(d) = self.peek() {
                    if d.is_ascii_alphabetic() {
                        self.at += 1;
                    } else {
                        break;
                    }
                }
                let cmd: String = self.chars[start..self.at].iter().collect();
                if cmd == "right" {
                    // Stop before consuming the delimiter char
                    break;
                }
                // Backtrack and let regular atom parser handle it
                self.at = at_mark;
            }
            if let Some(atom) = self.parse_atom() {
                let with_scripts = self.attach_scripts(atom);
                items.push(with_scripts);
            }
        }
        items
    }

    fn parse_environment(&mut self, env: &str) -> Math {
        let end_tag = format!(r"\end{{{env}}}");
        let mut rows: Vec<Vec<Math>> = vec![vec![]];
        let mut current_cell: Vec<Math> = Vec::new();

        while let Some(c) = self.peek_non_ws() {
            if c == '\\' {
                let at_mark = self.at;
                if self.starts_with(&end_tag) {
                    self.at += end_tag.len();
                    break;
                }
                if self.peek_at(self.at + 1) == Some('\\') {
                    // Row break `\\`
                    self.at += 2;
                    rows.last_mut().expect("a row").push(clean_row(std::mem::take(&mut current_cell)));
                    rows.push(vec![]);
                    continue;
                }
                self.at = at_mark;
            }
            if c == '&' {
                self.at += 1;
                rows.last_mut().expect("a row").push(clean_row(std::mem::take(&mut current_cell)));
                continue;
            }

            if let Some(atom) = self.parse_atom() {
                let with_scripts = self.attach_scripts(atom);
                current_cell.push(with_scripts);
            }
        }

        if !current_cell.is_empty() || rows.len() == 1 {
            rows.last_mut().expect("a row").push(clean_row(current_cell));
        }

        // Clean trailing empty rows
        if rows.last().is_some_and(|r| r.is_empty() || r.len() == 1 && matches!(r[0], Math::Row(ref v) if v.is_empty())) && rows.len() > 1 {
            rows.pop();
        }

        match env {
            "pmatrix" => Math::Delimited {
                open: "(".into(),
                close: ")".into(),
                sep: "|".into(),
                items: vec![Math::Matrix(rows)],
            },
            "bmatrix" => Math::Delimited {
                open: "[".into(),
                close: "]".into(),
                sep: "|".into(),
                items: vec![Math::Matrix(rows)],
            },
            "Bmatrix" => Math::Delimited {
                open: "{".into(),
                close: "}".into(),
                sep: "|".into(),
                items: vec![Math::Matrix(rows)],
            },
            "vmatrix" => Math::Delimited {
                open: "|".into(),
                close: "|".into(),
                sep: "|".into(),
                items: vec![Math::Matrix(rows)],
            },
            "aligned" | "align" => {
                let eq_rows: Vec<Math> = rows
                    .into_iter()
                    .map(|r| {
                        let mut row_items = Vec::new();
                        for (i, cell) in r.into_iter().enumerate() {
                            if i > 0 {
                                row_items.push(Math::Text("&".into()));
                            }
                            row_items.push(cell);
                        }
                        clean_row(row_items)
                    })
                    .collect();
                Math::EqArray(eq_rows)
            }
            _ => Math::Matrix(rows),
        }
    }

    fn starts_with(&self, s: &str) -> bool {
        let s_chars: Vec<char> = s.chars().collect();
        if self.at + s_chars.len() > self.chars.len() {
            return false;
        }
        for (i, &sc) in s_chars.iter().enumerate() {
            if self.chars[self.at + i] != sc {
                return false;
            }
        }
        true
    }

    fn attach_scripts(&mut self, mut base: Math) -> Math {
        while let Some(c) = self.peek_non_ws() {
            if c == '_' {
                self.at += 1;
                let sub = self.parse_required_arg();
                base = add_script(base, false, sub);
            } else if c == '^' {
                self.at += 1;
                let sup = self.parse_required_arg();
                base = add_script(base, true, sup);
            } else {
                break;
            }
        }
        base
    }

    fn parse_required_arg(&mut self) -> Math {
        self.skip_ws();
        let Some(c) = self.peek() else { return Math::Row(Vec::new()) };
        if c == '{' {
            self.at += 1;
            let inner = self.parse_row(&['}']);
            if self.peek() == Some('}') {
                self.at += 1;
            }
            clean_row(inner)
        } else if c == '\\' {
            self.at += 1;
            self.parse_command()
        } else {
            self.at += 1;
            Math::Text(c.to_string())
        }
    }

    fn parse_required_arg_text(&mut self) -> String {
        self.skip_ws();
        let Some(c) = self.peek() else { return String::new() };
        if c == '{' {
            self.at += 1;
            let start = self.at;
            while let Some(d) = self.peek() {
                if d == '}' {
                    break;
                }
                self.at += 1;
            }
            let s: String = self.chars[start..self.at].iter().collect();
            if self.peek() == Some('}') {
                self.at += 1;
            }
            s
        } else {
            self.at += 1;
            c.to_string()
        }
    }
}

fn clean_row(items: Vec<Math>) -> Math {
    row_of(merge_text(items))
}

fn row_of(mut items: Vec<Math>) -> Math {
    if items.len() == 1 {
        items.remove(0)
    } else {
        Math::Row(items)
    }
}

fn merge_text(items: Vec<Math>) -> Vec<Math> {
    let mut out: Vec<Math> = Vec::new();
    for item in items {
        match (out.last_mut(), item) {
            (Some(Math::Text(a)), Math::Text(b)) => a.push_str(&b),
            (_, item) => out.push(item),
        }
    }
    out
}

fn add_script(base: Math, sup: bool, arg: Math) -> Math {
    match base {
        Math::Script { base, sub, sup: old_sup } if (sup && old_sup.is_none()) || (!sup && sub.is_none()) => {
            if sup {
                Math::Script { base, sub, sup: Some(Box::new(arg)) }
            } else {
                Math::Script { base, sub: Some(Box::new(arg)), sup: old_sup }
            }
        }
        Math::Nary { op, sub, sup: old_sup, body, stacked } if (sup && old_sup.is_none()) || (!sup && sub.is_none()) => {
            if sup {
                Math::Nary { op, sub, sup: Some(Box::new(arg)), body, stacked }
            } else {
                Math::Nary { op, sub: Some(Box::new(arg)), sup: old_sup, body, stacked }
            }
        }
        base if sup => Math::Script { base: Box::new(base), sub: None, sup: Some(Box::new(arg)) },
        base => Math::Script { base: Box::new(base), sub: Some(Box::new(arg)), sup: None },
    }
}

pub(crate) fn lookup_latex_symbol(cmd: &str) -> Option<&'static str> {
    match cmd {
        // Lowercase Greek
        "alpha" => Some("α"),
        "beta" => Some("β"),
        "gamma" => Some("γ"),
        "delta" => Some("δ"),
        "epsilon" | "varepsilon" => Some("ε"),
        "zeta" => Some("ζ"),
        "eta" => Some("η"),
        "theta" | "vartheta" => Some("θ"),
        "iota" => Some("ι"),
        "kappa" => Some("κ"),
        "lambda" => Some("λ"),
        "mu" => Some("μ"),
        "nu" => Some("ν"),
        "xi" => Some("ξ"),
        "pi" | "varpi" => Some("π"),
        "rho" | "varrho" => Some("ρ"),
        "sigma" | "varsigma" => Some("σ"),
        "tau" => Some("τ"),
        "upsilon" => Some("υ"),
        "phi" | "varphi" => Some("φ"),
        "chi" => Some("χ"),
        "psi" => Some("ψ"),
        "omega" => Some("ω"),

        // Uppercase Greek
        "Gamma" => Some("Γ"),
        "Delta" => Some("Δ"),
        "Theta" => Some("Θ"),
        "Lambda" => Some("Λ"),
        "Xi" => Some("Ξ"),
        "Pi" => Some("Π"),
        "Sigma" => Some("Σ"),
        "Upsilon" => Some("Υ"),
        "Phi" => Some("Φ"),
        "Psi" => Some("Ψ"),
        "Omega" => Some("Ω"),

        // Operators & Relations
        "pm" => Some("±"),
        "mp" => Some("∓"),
        "times" => Some("×"),
        "div" => Some("÷"),
        "cdot" => Some("·"),
        "circ" => Some("∘"),
        "bullet" => Some("∙"),
        "le" | "leq" => Some("≤"),
        "ge" | "geq" => Some("≥"),
        "ne" | "neq" => Some("≠"),
        "approx" => Some("≈"),
        "equiv" => Some("≡"),
        "sim" => Some("∼"),
        "infty" => Some("∞"),
        "partial" => Some("∂"),
        "nabla" => Some("∇"),
        "in" => Some("∈"),
        "notin" => Some("∉"),
        "subset" => Some("⊂"),
        "subseteq" => Some("⊆"),
        "cup" => Some("∪"),
        "cap" => Some("∩"),
        "to" | "rightarrow" => Some("→"),
        "leftarrow" => Some("←"),
        "Rightarrow" => Some("⇒"),
        "Leftarrow" => Some("⇐"),
        "Leftrightarrow" => Some("⇔"),
        "forall" => Some("∀"),
        "exists" => Some("∃"),
        "neg" => Some("¬"),
        "dots" | "cdots" | "ldots" => Some("…"),

        _ => None,
    }
}
