// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork (contributed under the ibisOffice CLA, https://github.com/ibisjp/ibis-office/issues/226)
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Design-system resolution, token loading, and DADS CSS token import for Markdown single-source generation.
//!
//! ## Folder convention
//!
//! A design system is stored in a directory named `<name>` under `DesignSystems/`, resolved in order:
//! 1. **Project folder**: `<project-dir>/DesignSystems/<name>/` (inside the opened project/document directory,
//!    so teams can Git-manage design systems alongside their Markdown documents).
//! 2. **System folder**: `<app-data>/ibisOffice/DesignSystems/<name>/` (sibling to `ChartTemplates/`,
//!    overridable via `IBIS_DATA_DIR`).
//!
//! Each `<name>/` directory contains:
//! - `tokens.yaml` (required): colours, fonts, typography scale, spacing, and optional PPTX `slide_size`.
//! - `layouts.yaml` (optional, format-only in this increment): named slide layout declarations.
//! - `styles.yaml` (optional, format-only in this increment): DOCX style mapping declarations.
//! - `base.pptx` / `base.docx` (optional, allowed to exist; not applied as generation bases in this increment).
//!
//! ## DADS importer input format choice
//!
//! The Digital Agency Design System (DADS, `digital-go-jp/design-tokens`, MIT) authors tokens in
//! Tokens Studio JSON and publishes Style Dictionary-built CSS custom properties (`tokens.css` /
//! `tokens-simple.css`) in `@digital-go-jp/design-tokens` (v2.1.0).
//!
//! **Chosen format: built CSS custom properties (`tokens.css` / `tokens-simple.css`).**
//!
//! **Rationale:**
//! - Style Dictionary has already flattened the multi-file Tokens Studio hierarchy and resolved alias
//!   chains (`{color.primitive.blue.900}` → concrete CSS values), avoiding a brittle re-implementation
//!   of Tokens Studio's `$themes.json` / `$metadata.json` merge semantics.
//! - Parsing CSS custom properties (`--name: value;`, including `var(--...)` references) requires zero
//!   extra crate dependencies and accepts a single self-contained file directly from the published npm
//!   package without Figma or Node build tooling.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use ibis_doc_model::{
    Emu, EmuSize, FlowStyle, FlowStyles, FontSet, LineHeight, ParaFormat, Rgb, RunFormat, StyleKind,
    ThemeFonts,
};
use serde::Deserialize;

use crate::Error;

/// Directory name holding design systems in both project folders and `<app-data>/ibisOffice/`.
pub const DESIGN_SYSTEMS_DIR_NAME: &str = "DesignSystems";
pub const TOKENS_FILE_NAME: &str = "tokens.yaml";
pub const LAYOUTS_FILE_NAME: &str = "layouts.yaml";
pub const STYLES_FILE_NAME: &str = "styles.yaml";
pub const BASE_PPTX_FILE_NAME: &str = "base.pptx";
pub const BASE_DOCX_FILE_NAME: &str = "base.docx";

const EMU_PER_PT: i64 = 12_700;
const EMU_PER_INCH: i64 = 914_400;
const EMU_PER_MM: f64 = 36_000.0;

/// Where a design system was discovered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesignSystemScope {
    Project,
    System,
}

/// Summary entry returned when scanning available design systems.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesignSystemInfo {
    pub name: String,
    pub path: PathBuf,
    pub scope: DesignSystemScope,
}

/// Metadata recording the origin of an imported token set.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct TokenSourceMeta {
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

/// Colour tokens in `tokens.yaml`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct ColorTokens {
    #[serde(default)]
    pub primary: Option<String>,
    #[serde(default)]
    pub secondary: Option<String>,
    #[serde(default)]
    pub surface: Option<String>,
    #[serde(default)]
    pub surface_subtle: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub text_muted: Option<String>,
    #[serde(default)]
    pub link: Option<String>,
    #[serde(default)]
    pub visited_link: Option<String>,
    /// Ordered accent palette mapped to OOXML `accent1`..`accent6`.
    #[serde(default)]
    pub accents: Vec<String>,
    /// Optional chart series colour sequence override (falls back to `accents`).
    #[serde(default)]
    pub chart: Vec<String>,
}

/// Typography size scale (values in points; YAML also accepts `pt`, `px`, `rem` strings).
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct TypeScaleTokens {
    #[serde(default, deserialize_with = "de_opt_pt")]
    pub body: Option<f64>,
    #[serde(default, deserialize_with = "de_opt_pt")]
    pub h1: Option<f64>,
    #[serde(default, deserialize_with = "de_opt_pt")]
    pub h2: Option<f64>,
    #[serde(default, deserialize_with = "de_opt_pt")]
    pub h3: Option<f64>,
    #[serde(default, deserialize_with = "de_opt_pt")]
    pub h4: Option<f64>,
    #[serde(default, deserialize_with = "de_opt_pt")]
    pub h5: Option<f64>,
    #[serde(default, deserialize_with = "de_opt_pt")]
    pub h6: Option<f64>,
    #[serde(default, deserialize_with = "de_opt_pt")]
    pub slide_cover_title: Option<f64>,
    #[serde(default, deserialize_with = "de_opt_pt")]
    pub slide_title: Option<f64>,
    #[serde(default, deserialize_with = "de_opt_pt")]
    pub slide_body: Option<f64>,
}

/// Typography tokens in `tokens.yaml`.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct TypographyTokens {
    #[serde(default)]
    pub font_family: Option<String>,
    #[serde(default)]
    pub heading_font_family: Option<String>,
    #[serde(default)]
    pub east_asian_font: Option<String>,
    #[serde(default)]
    pub mono_font_family: Option<String>,
    #[serde(default)]
    pub sizes: TypeScaleTokens,
    #[serde(default)]
    pub line_height: Option<f64>,
}

/// Spacing tokens in `tokens.yaml` (points; YAML also accepts `pt`, `px`, `rem`, `mm` strings).
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct SpacingTokens {
    #[serde(default, deserialize_with = "de_opt_pt")]
    pub paragraph_after: Option<f64>,
    #[serde(default, deserialize_with = "de_opt_pt")]
    pub heading_before: Option<f64>,
    #[serde(default, deserialize_with = "de_opt_pt")]
    pub heading_after: Option<f64>,
    #[serde(default, deserialize_with = "de_pt_map")]
    pub scale: BTreeMap<String, f64>,
}

/// Optional PPTX slide size token in `tokens.yaml`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct SlideSizeToken {
    /// Preset aspect ratio: `"16:9"` (widescreen) or `"4:3"` (standard).
    #[serde(default)]
    pub preset: Option<String>,
    /// Explicit width (e.g. `"338.67mm"`, `"13.333in"`, `"960pt"`, or EMU integer).
    #[serde(default, deserialize_with = "de_opt_string_or_number")]
    pub width: Option<String>,
    /// Explicit height (e.g. `"190.5mm"`, `"7.5in"`, `"540pt"`, or EMU integer).
    #[serde(default, deserialize_with = "de_opt_string_or_number")]
    pub height: Option<String>,
}

/// Full `tokens.yaml` payload.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct DesignTokens {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub source: Option<TokenSourceMeta>,
    #[serde(default)]
    pub colors: ColorTokens,
    #[serde(default)]
    pub typography: TypographyTokens,
    #[serde(default)]
    pub spacing: SpacingTokens,
    #[serde(default)]
    pub slide_size: Option<SlideSizeToken>,
}

/// Slide layout declaration entry (`layouts.yaml`, format definition and loading only in this increment).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct LayoutEntry {
    pub name: String,
    #[serde(default)]
    pub master_layout: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// `layouts.yaml` schema (loaded and validated; slide-layout mapping is deferred to a later increment).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct LayoutsConfig {
    #[serde(default)]
    pub layouts: Vec<LayoutEntry>,
}

/// DOCX style mapping entry (`styles.yaml`, format definition and loading only in this increment).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct StyleMappingEntry {
    #[serde(default)]
    pub docx_style: Option<String>,
    #[serde(default)]
    pub based_on: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// `styles.yaml` schema (loaded and validated; DOCX style remapping is deferred to a later increment).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct StylesConfig {
    #[serde(default)]
    pub styles: BTreeMap<String, StyleMappingEntry>,
}

/// A resolved, loaded design system directory.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignSystem {
    pub name: String,
    pub root: PathBuf,
    pub scope: DesignSystemScope,
    pub tokens: DesignTokens,
    /// Loaded from `layouts.yaml` if present (format-only in this increment).
    pub layouts: Option<LayoutsConfig>,
    /// Loaded from `styles.yaml` if present (format-only in this increment).
    pub styles: Option<StylesConfig>,
    /// Path to optional `base.pptx` if present (not applied as a generation base in this increment).
    pub base_pptx: Option<PathBuf>,
    /// Path to optional `base.docx` if present (not applied as a generation base in this increment).
    pub base_docx: Option<PathBuf>,
}

/// Concrete OOXML theme, DOCX style, PPTX typography, and chart colour values derived from [`DesignTokens`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesignSystemTheme {
    /// OOXML 12-slot theme colour scheme sorted by scheme name (`accent1`..`accent6`, `dk1`, `dk2`, `folHlink`, `hlink`, `lt1`, `lt2`).
    pub theme_colors: Vec<(String, Rgb)>,
    /// OOXML theme fonts (`majorFont` / `minorFont`).
    pub theme_fonts: ThemeFonts,
    /// Ordered 6-colour sequence used for chart series (`accent1`..`accent6`).
    pub chart_colors: Vec<Rgb>,
    pub primary: Rgb,
    pub secondary: Rgb,
    pub text: Rgb,
    pub surface: Rgb,
    pub surface_subtle: Rgb,
    pub body_font: Option<String>,
    pub heading_font: Option<String>,
    pub east_asian_font: Option<String>,
    /// Body text size in hundredths of a point.
    pub body_size_hp: u32,
    /// Heading H1..H6 sizes in hundredths of a point.
    pub heading_sizes_hp: [u32; 6],
    /// Slide cover (first slide) title size in hundredths of a point.
    pub slide_cover_title_hp: u32,
    /// Slide content title size in hundredths of a point.
    pub slide_title_hp: u32,
    /// Slide body text size in hundredths of a point.
    pub slide_body_hp: u32,
    pub line_height: Option<LineHeight>,
    pub paragraph_after: Option<Emu>,
    pub heading_before: Option<Emu>,
    pub heading_after: Option<Emu>,
    pub slide_size: Option<EmuSize>,
}

// ============================================================================
// Folder discovery & resolution
// ============================================================================

/// Returns the default system design-systems directory (`<app-data>/ibisOffice/DesignSystems`),
/// mirroring `ibis_office_base::app_data::base()` (`IBIS_DATA_DIR` first, then platform data dir).
#[cfg(not(target_family = "wasm"))]
pub fn default_system_dir() -> Option<PathBuf> {
    let env = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = env("IBIS_DATA_DIR").or_else(|| {
        #[cfg(target_os = "macos")]
        {
            env("HOME").map(|home| home.join("Library/Application Support"))
        }
        #[cfg(target_os = "windows")]
        {
            env("APPDATA")
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            env("XDG_DATA_HOME").or_else(|| env("HOME").map(|home| home.join(".local/share")))
        }
    })?;
    Some(base.join("ibisOffice").join(DESIGN_SYSTEMS_DIR_NAME))
}

#[cfg(target_family = "wasm")]
pub fn default_system_dir() -> Option<PathBuf> {
    None
}

/// Validates a design system name so it cannot escape `DesignSystems/`.
pub fn validate_name(name: &str) -> Result<&str, Error> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|', '\0'])
    {
        return Err(Error::new(format!(
            "無効なデザインシステム名です (invalid design system name): '{name}'"
        )));
    }
    Ok(trimmed)
}

/// Resolves a design system folder by name: project `<project_dir>/DesignSystems/<name>/` first,
/// then system `<system_dir>/<name>/` (defaulting to [`default_system_dir`]).
///
/// Never falls back to a default design; if unresolved, returns an error listing every searched path.
#[cfg(not(target_family = "wasm"))]
pub fn resolve_dir(
    name: &str,
    project_dir: Option<&Path>,
    system_dir: Option<&Path>,
) -> Result<(PathBuf, DesignSystemScope), Error> {
    let name = validate_name(name)?;
    let mut searched: Vec<(PathBuf, DesignSystemScope)> = Vec::new();
    if let Some(proj) = project_dir {
        searched.push((proj.join(DESIGN_SYSTEMS_DIR_NAME).join(name), DesignSystemScope::Project));
    }
    let effective_system = system_dir.map(Path::to_path_buf).or_else(default_system_dir);
    if let Some(sys) = effective_system {
        let candidate = sys.join(name);
        if !searched.iter().any(|(p, _)| p == &candidate) {
            searched.push((candidate, DesignSystemScope::System));
        }
    }

    for (candidate, scope) in &searched {
        if candidate.is_dir() {
            return Ok((candidate.clone(), *scope));
        }
    }

    let locations = if searched.is_empty() {
        "(検索可能な場所がありません / no search directories configured)".to_string()
    } else {
        searched
            .iter()
            .map(|(p, _)| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    Err(Error::new(format!(
        "デザインシステム '{name}' が見つかりません (design system '{name}' not found). 検索した場所 (searched): {locations}"
    )))
}

#[cfg(target_family = "wasm")]
pub fn resolve_dir(
    name: &str,
    _project_dir: Option<&Path>,
    _system_dir: Option<&Path>,
) -> Result<(PathBuf, DesignSystemScope), Error> {
    let name = validate_name(name)?;
    Err(Error::new(format!(
        "デザインシステム '{name}' はこの環境（WASM）のファイルシステムから読み込めません"
    )))
}

/// Scans project and system `DesignSystems/` folders in `ChartTemplates`-style, returning sorted entries
/// with project-level design systems taking precedence over system-level ones of the same name.
#[cfg(not(target_family = "wasm"))]
pub fn list(project_dir: Option<&Path>, system_dir: Option<&Path>) -> Vec<DesignSystemInfo> {
    let mut roots: Vec<(PathBuf, DesignSystemScope)> = Vec::new();
    if let Some(proj) = project_dir {
        roots.push((proj.join(DESIGN_SYSTEMS_DIR_NAME), DesignSystemScope::Project));
    }
    if let Some(sys) = system_dir.map(Path::to_path_buf).or_else(default_system_dir) {
        if !roots.iter().any(|(p, _)| p == &sys) {
            roots.push((sys, DesignSystemScope::System));
        }
    }

    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (root, scope) in roots {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
                continue;
            };
            if validate_name(&name).is_err() || !path.join(TOKENS_FILE_NAME).is_file() {
                continue;
            }
            if seen.insert(name.clone()) {
                out.push(DesignSystemInfo { name, path, scope });
            }
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

#[cfg(target_family = "wasm")]
pub fn list(_project_dir: Option<&Path>, _system_dir: Option<&Path>) -> Vec<DesignSystemInfo> {
    Vec::new()
}

/// Resolves and loads a design system by name.
#[cfg(not(target_family = "wasm"))]
pub fn load(
    name: &str,
    project_dir: Option<&Path>,
    system_dir: Option<&Path>,
) -> Result<DesignSystem, Error> {
    let (root, scope) = resolve_dir(name, project_dir, system_dir)?;
    load_from_dir(name.trim(), &root, scope)
}

#[cfg(target_family = "wasm")]
pub fn load(
    name: &str,
    project_dir: Option<&Path>,
    system_dir: Option<&Path>,
) -> Result<DesignSystem, Error> {
    let _ = resolve_dir(name, project_dir, system_dir)?;
    unreachable!()
}

/// Loads a design system directly from a resolved folder path.
#[cfg(not(target_family = "wasm"))]
pub fn load_from_dir(name: &str, root: &Path, scope: DesignSystemScope) -> Result<DesignSystem, Error> {
    let tokens_path = root.join(TOKENS_FILE_NAME);
    if !tokens_path.is_file() {
        return Err(Error::new(format!(
            "デザインシステム '{name}' に {TOKENS_FILE_NAME} がありません: {}",
            tokens_path.display()
        )));
    }
    let tokens_raw = std::fs::read_to_string(&tokens_path).map_err(|e| {
        Error::new(format!("{}: {e}", tokens_path.display()))
    })?;
    let tokens = DesignTokens::from_yaml(&tokens_raw).map_err(|e| {
        Error::new(format!("{}: {e}", tokens_path.display()))
    })?;

    let layouts_path = root.join(LAYOUTS_FILE_NAME);
    let layouts = if layouts_path.is_file() {
        let raw = std::fs::read_to_string(&layouts_path)
            .map_err(|e| Error::new(format!("{}: {e}", layouts_path.display())))?;
        let parsed = LayoutsConfig::from_yaml(&raw)
            .map_err(|e| Error::new(format!("{}: {e}", layouts_path.display())))?;
        Some(parsed)
    } else {
        None
    };

    let styles_path = root.join(STYLES_FILE_NAME);
    let styles = if styles_path.is_file() {
        let raw = std::fs::read_to_string(&styles_path)
            .map_err(|e| Error::new(format!("{}: {e}", styles_path.display())))?;
        let parsed = StylesConfig::from_yaml(&raw)
            .map_err(|e| Error::new(format!("{}: {e}", styles_path.display())))?;
        Some(parsed)
    } else {
        None
    };

    let base_pptx = root.join(BASE_PPTX_FILE_NAME);
    let base_pptx = base_pptx.is_file().then_some(base_pptx);
    let base_docx = root.join(BASE_DOCX_FILE_NAME);
    let base_docx = base_docx.is_file().then_some(base_docx);

    Ok(DesignSystem {
        name: name.to_string(),
        root: root.to_path_buf(),
        scope,
        tokens,
        layouts,
        styles,
        base_pptx,
        base_docx,
    })
}

impl DesignSystem {
    /// Computes the OOXML theme, DOCX style, PPTX typography, and chart colour mapping for this design system.
    pub fn theme(&self) -> Result<DesignSystemTheme, Error> {
        self.tokens.to_theme()
    }
}

// ============================================================================
// Parsing, validation & serialization
// ============================================================================

impl LayoutsConfig {
    pub fn from_yaml(yaml: &str) -> Result<Self, Error> {
        if yaml.trim().is_empty() {
            return Ok(Self::default());
        }
        serde_saphyr::from_str(yaml)
            .map_err(|e| Error::new(format!("{LAYOUTS_FILE_NAME} の YAML が不正です: {e}")))
    }
}

impl StylesConfig {
    pub fn from_yaml(yaml: &str) -> Result<Self, Error> {
        if yaml.trim().is_empty() {
            return Ok(Self::default());
        }
        serde_saphyr::from_str(yaml)
            .map_err(|e| Error::new(format!("{STYLES_FILE_NAME} の YAML が不正です: {e}")))
    }
}

impl DesignTokens {
    /// Parses `tokens.yaml` and validates all colour and slide-size values.
    pub fn from_yaml(yaml: &str) -> Result<Self, Error> {
        let tokens: Self = serde_saphyr::from_str(yaml)
            .map_err(|e| Error::new(format!("{TOKENS_FILE_NAME} の YAML が不正です: {e}")))?;
        // Validate eagerly so malformed hex codes or slide sizes fail fast.
        let _ = tokens.to_theme()?;
        Ok(tokens)
    }

    /// Serializes tokens into canonical `tokens.yaml` text.
    pub fn to_yaml(&self) -> String {
        let mut out = String::new();
        if let Some(name) = &self.name {
            out.push_str(&format!("name: {}\n", yaml_str(name)));
        }
        if let Some(src) = &self.source {
            out.push_str("source:\n");
            if let Some(v) = &src.repository {
                out.push_str(&format!("  repository: {}\n", yaml_str(v)));
            }
            if let Some(v) = &src.package {
                out.push_str(&format!("  package: {}\n", yaml_str(v)));
            }
            if let Some(v) = &src.version {
                out.push_str(&format!("  version: {}\n", yaml_str(v)));
            }
            if let Some(v) = &src.license {
                out.push_str(&format!("  license: {}\n", yaml_str(v)));
            }
            if let Some(v) = &src.note {
                out.push_str(&format!("  note: {}\n", yaml_str(v)));
            }
        }
        out.push_str("colors:\n");
        let push_opt_str = |buf: &mut String, indent: &str, key: &str, val: &Option<String>| {
            if let Some(v) = val {
                buf.push_str(&format!("{indent}{key}: {}\n", yaml_str(v)));
            }
        };
        push_opt_str(&mut out, "  ", "primary", &self.colors.primary);
        push_opt_str(&mut out, "  ", "secondary", &self.colors.secondary);
        push_opt_str(&mut out, "  ", "surface", &self.colors.surface);
        push_opt_str(&mut out, "  ", "surface_subtle", &self.colors.surface_subtle);
        push_opt_str(&mut out, "  ", "text", &self.colors.text);
        push_opt_str(&mut out, "  ", "text_muted", &self.colors.text_muted);
        push_opt_str(&mut out, "  ", "link", &self.colors.link);
        push_opt_str(&mut out, "  ", "visited_link", &self.colors.visited_link);
        if !self.colors.accents.is_empty() {
            out.push_str("  accents:\n");
            for c in &self.colors.accents {
                out.push_str(&format!("    - {}\n", yaml_str(c)));
            }
        }
        if !self.colors.chart.is_empty() {
            out.push_str("  chart:\n");
            for c in &self.colors.chart {
                out.push_str(&format!("    - {}\n", yaml_str(c)));
            }
        }

        out.push_str("typography:\n");
        push_opt_str(&mut out, "  ", "font_family", &self.typography.font_family);
        push_opt_str(&mut out, "  ", "heading_font_family", &self.typography.heading_font_family);
        push_opt_str(&mut out, "  ", "east_asian_font", &self.typography.east_asian_font);
        push_opt_str(&mut out, "  ", "mono_font_family", &self.typography.mono_font_family);
        let push_opt_num = |buf: &mut String, indent: &str, key: &str, val: Option<f64>| {
            if let Some(v) = val {
                buf.push_str(&format!("{indent}{key}: {}\n", fmt_num(v)));
            }
        };
        let s = &self.typography.sizes;
        if s.body.is_some()
            || s.h1.is_some()
            || s.h2.is_some()
            || s.h3.is_some()
            || s.h4.is_some()
            || s.h5.is_some()
            || s.h6.is_some()
            || s.slide_cover_title.is_some()
            || s.slide_title.is_some()
            || s.slide_body.is_some()
        {
            out.push_str("  sizes:\n");
            push_opt_num(&mut out, "    ", "body", s.body);
            push_opt_num(&mut out, "    ", "h1", s.h1);
            push_opt_num(&mut out, "    ", "h2", s.h2);
            push_opt_num(&mut out, "    ", "h3", s.h3);
            push_opt_num(&mut out, "    ", "h4", s.h4);
            push_opt_num(&mut out, "    ", "h5", s.h5);
            push_opt_num(&mut out, "    ", "h6", s.h6);
            push_opt_num(&mut out, "    ", "slide_cover_title", s.slide_cover_title);
            push_opt_num(&mut out, "    ", "slide_title", s.slide_title);
            push_opt_num(&mut out, "    ", "slide_body", s.slide_body);
        }
        push_opt_num(&mut out, "  ", "line_height", self.typography.line_height);

        if self.spacing.paragraph_after.is_some()
            || self.spacing.heading_before.is_some()
            || self.spacing.heading_after.is_some()
            || !self.spacing.scale.is_empty()
        {
            out.push_str("spacing:\n");
            push_opt_num(&mut out, "  ", "paragraph_after", self.spacing.paragraph_after);
            push_opt_num(&mut out, "  ", "heading_before", self.spacing.heading_before);
            push_opt_num(&mut out, "  ", "heading_after", self.spacing.heading_after);
            if !self.spacing.scale.is_empty() {
                out.push_str("  scale:\n");
                for (k, v) in &self.spacing.scale {
                    out.push_str(&format!("    {k}: {}\n", fmt_num(*v)));
                }
            }
        }

        if let Some(sz) = &self.slide_size {
            out.push_str("slide_size:\n");
            push_opt_str(&mut out, "  ", "preset", &sz.preset);
            push_opt_str(&mut out, "  ", "width", &sz.width);
            push_opt_str(&mut out, "  ", "height", &sz.height);
        }
        out
    }

    /// Maps design tokens into OOXML theme colours, theme fonts, DOCX styles, PPTX sizes, and chart palette.
    pub fn to_theme(&self) -> Result<DesignSystemTheme, Error> {
        let parse_opt = |field: &str, value: &Option<String>| -> Result<Option<Rgb>, Error> {
            match value {
                Some(raw) => parse_hex_color(raw)
                    .map(Some)
                    .ok_or_else(|| Error::new(format!("tokens.yaml の色 '{field}' が不正です: '{raw}'"))),
                None => Ok(None),
            }
        };

        let parsed_accents: Vec<Rgb> = self
            .colors
            .accents
            .iter()
            .enumerate()
            .map(|(i, raw)| {
                parse_hex_color(raw).ok_or_else(|| {
                    Error::new(format!("tokens.yaml の色 'colors.accents[{i}]' が不正です: '{raw}'"))
                })
            })
            .collect::<Result<_, _>>()?;

        let parsed_chart: Vec<Rgb> = self
            .colors
            .chart
            .iter()
            .enumerate()
            .map(|(i, raw)| {
                parse_hex_color(raw).ok_or_else(|| {
                    Error::new(format!("tokens.yaml の色 'colors.chart[{i}]' が不正です: '{raw}'"))
                })
            })
            .collect::<Result<_, _>>()?;

        let primary = parse_opt("colors.primary", &self.colors.primary)?
            .or_else(|| parsed_accents.first().copied())
            .or_else(|| parsed_chart.first().copied())
            .unwrap_or(Rgb(0x0017C1));
        let text = parse_opt("colors.text", &self.colors.text)?.unwrap_or(Rgb(0x1A1A1C));
        let text_muted = parse_opt("colors.text_muted", &self.colors.text_muted)?;
        let secondary = parse_opt("colors.secondary", &self.colors.secondary)?
            .or(text_muted)
            .unwrap_or(primary);
        let surface = parse_opt("colors.surface", &self.colors.surface)?.unwrap_or(Rgb(0xFFFFFF));
        let surface_subtle =
            parse_opt("colors.surface_subtle", &self.colors.surface_subtle)?.unwrap_or(Rgb(0xE8F1FE));
        let link = parse_opt("colors.link", &self.colors.link)?.unwrap_or(primary);
        let visited_link =
            parse_opt("colors.visited_link", &self.colors.visited_link)?.unwrap_or(Rgb(0x6F2DBD));

        // Chart sequence drives OOXML accent1..accent6 so native charts (whose series reference
        // `a:schemeClr val="accent1"`..`accent6`) automatically adopt the design system's palette.
        const FALLBACK_ACCENTS: [Rgb; 6] = [
            Rgb(0x0017C1),
            Rgb(0x00825A),
            Rgb(0xD93025),
            Rgb(0xB76E00),
            Rgb(0x0066B3),
            Rgb(0x6F2DBD),
        ];
        let mut chart_colors: Vec<Rgb> = if !parsed_chart.is_empty() {
            parsed_chart
        } else if !parsed_accents.is_empty() {
            parsed_accents
        } else {
            vec![primary]
        };
        if chart_colors.len() < 6 {
            for &fallback in &FALLBACK_ACCENTS {
                if chart_colors.len() >= 6 {
                    break;
                }
                if !chart_colors.contains(&fallback) {
                    chart_colors.push(fallback);
                }
            }
            while chart_colors.len() < 6 {
                chart_colors.push(primary);
            }
        }
        chart_colors.truncate(6);

        let mut theme_colors = vec![
            ("dk1".to_string(), text),
            ("lt1".to_string(), surface),
            ("dk2".to_string(), secondary),
            ("lt2".to_string(), surface_subtle),
            ("accent1".to_string(), chart_colors[0]),
            ("accent2".to_string(), chart_colors[1]),
            ("accent3".to_string(), chart_colors[2]),
            ("accent4".to_string(), chart_colors[3]),
            ("accent5".to_string(), chart_colors[4]),
            ("accent6".to_string(), chart_colors[5]),
            ("hlink".to_string(), link),
            ("folHlink".to_string(), visited_link),
        ];
        theme_colors.sort_by(|a, b| a.0.cmp(&b.0));

        let body_font = self
            .typography
            .font_family
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let heading_font = self
            .typography
            .heading_font_family
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| body_font.clone());
        let east_asian_font = self
            .typography
            .east_asian_font
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| body_font.clone());

        let theme_fonts = ThemeFonts {
            major: FontSet {
                latin: heading_font.clone(),
                east_asian: heading_font.clone().or_else(|| east_asian_font.clone()),
                complex: None,
            },
            minor: FontSet {
                latin: body_font.clone(),
                east_asian: east_asian_font.clone(),
                complex: None,
            },
        };

        let sizes = &self.typography.sizes;
        let body_pt = sizes.body.unwrap_or(11.0);
        let h1_pt = sizes.h1.unwrap_or(20.0);
        let h2_pt = sizes.h2.unwrap_or(16.0);
        let h3_pt = sizes.h3.unwrap_or(13.5);
        let h4_pt = sizes.h4.unwrap_or(12.0);
        let h5_pt = sizes.h5.unwrap_or(11.0);
        let h6_pt = sizes.h6.unwrap_or(body_pt);
        let slide_cover_title_pt = sizes.slide_cover_title.unwrap_or(40.0);
        let slide_title_pt = sizes.slide_title.unwrap_or(28.0);
        let slide_body_pt = sizes.slide_body.unwrap_or(18.0);

        let to_hp = |pt: f64| -> u32 { (pt.clamp(4.0, 144.0) * 100.0).round() as u32 };

        let line_height = self
            .typography
            .line_height
            .filter(|lh| lh.is_finite() && *lh > 0.5 && *lh <= 4.0)
            .map(|lh| LineHeight::Auto((lh * 240.0).round() as u32));

        let pt_to_emu = |pt: f64| -> Emu { (pt.max(0.0) * EMU_PER_PT as f64).round() as Emu };
        let paragraph_after = self.spacing.paragraph_after.map(pt_to_emu);
        let heading_before = self.spacing.heading_before.map(pt_to_emu);
        let heading_after = self.spacing.heading_after.map(pt_to_emu);

        let slide_size = match &self.slide_size {
            Some(sz) => Some(parse_slide_size_token(sz)?),
            None => None,
        };

        Ok(DesignSystemTheme {
            theme_colors,
            theme_fonts,
            chart_colors,
            primary,
            secondary,
            text,
            surface,
            surface_subtle,
            body_font,
            heading_font,
            east_asian_font,
            body_size_hp: to_hp(body_pt),
            heading_sizes_hp: [
                to_hp(h1_pt),
                to_hp(h2_pt),
                to_hp(h3_pt),
                to_hp(h4_pt),
                to_hp(h5_pt),
                to_hp(h6_pt),
            ],
            slide_cover_title_hp: to_hp(slide_cover_title_pt),
            slide_title_hp: to_hp(slide_title_pt),
            slide_body_hp: to_hp(slide_body_pt),
            line_height,
            paragraph_after,
            heading_before,
            heading_after,
            slide_size,
        })
    }
}

impl DesignSystemTheme {
    /// Applies typography, colours, and spacing tokens to DOCX [`FlowStyles`].
    ///
    /// Modifies `defaults_run`, the default `Normal` paragraph style, and `Heading1`..`Heading6`
    /// (preserving `FlowStyles::defaults_para` so `save/docx.rs` writes all changes cleanly).
    pub fn apply_to_flow_styles(&self, styles: &mut FlowStyles) {
        if let Some(font) = &self.body_font {
            styles.defaults_run.font = Some(font.clone());
            styles.defaults_run.h_ansi_font = Some(font.clone());
        }
        if let Some(ea) = &self.east_asian_font {
            styles.defaults_run.east_asian_font = Some(ea.clone());
        }
        styles.defaults_run.size = Some(self.body_size_hp);
        styles.defaults_run.cs_size = Some(self.body_size_hp);
        styles.defaults_run.color = Some(self.text);

        if let Some(normal_id) = styles
            .default_paragraph
            .clone()
            .or_else(|| styles.paragraph_style_named("Normal").map(str::to_string))
            && let Some(st) = styles.styles.get_mut(&normal_id)
        {
            if let Some(font) = &self.body_font {
                st.run.font = Some(font.clone());
                st.run.h_ansi_font = Some(font.clone());
            }
            if let Some(ea) = &self.east_asian_font {
                st.run.east_asian_font = Some(ea.clone());
            }
            st.run.size = Some(self.body_size_hp);
            st.run.cs_size = Some(self.body_size_hp);
            st.run.color = Some(self.text);
            st.run.theme_color = None;
            if let Some(after) = self.paragraph_after {
                st.para.after = Some(after);
            }
            if let Some(lh) = self.line_height {
                st.para.line = Some(lh);
            }
        }

        let normal_id = styles.default_paragraph.clone();
        for level in 1..=6u8 {
            let name = format!("heading {level}");
            let id = styles
                .paragraph_style_named(&name)
                .map(str::to_string)
                .unwrap_or_else(|| format!("Heading{level}"));
            let size_hp = self.heading_sizes_hp[(level - 1) as usize];
            let heading_color = if level <= 2 { self.primary } else { self.secondary };
            let default_before = match level {
                1 => 14 * EMU_PER_PT,
                2 => 12 * EMU_PER_PT,
                3 => 10 * EMU_PER_PT,
                _ => 6 * EMU_PER_PT,
            };
            let default_after = if level <= 2 { 6 * EMU_PER_PT } else { 4 * EMU_PER_PT };
            let before = self.heading_before.unwrap_or(default_before);
            let after = self.heading_after.unwrap_or(default_after);

            let entry = styles.styles.entry(id).or_insert_with(|| FlowStyle {
                name: name.clone(),
                kind: StyleKind::Paragraph,
                based_on: normal_id.clone(),
                next: normal_id.clone(),
                quick: true,
                ui_priority: Some(u32::from(level)),
                auto_update: false,
                conditional: Vec::new(),
                num_id: None,
                para: ParaFormat {
                    keep_next: Some(true),
                    keep_lines: Some(true),
                    ..ParaFormat::default()
                },
                run: RunFormat::default(),
            });
            entry.para.before = Some(before);
            entry.para.after = Some(after);
            entry.para.keep_next = Some(true);
            entry.para.keep_lines = Some(true);
            entry.run.bold = Some(true);
            entry.run.size = Some(size_hp);
            entry.run.cs_size = Some(size_hp);
            entry.run.color = Some(heading_color);
            entry.run.theme_color = None;
            if let Some(font) = &self.heading_font {
                entry.run.font = Some(font.clone());
                entry.run.h_ansi_font = Some(font.clone());
            }
            if let Some(ea) = self.heading_font.as_ref().or(self.east_asian_font.as_ref()) {
                entry.run.east_asian_font = Some(ea.clone());
            }
        }
    }

    /// Look up a named theme colour (`accent1`, `dk1`, `lt2`, …).
    pub fn color(&self, scheme_name: &str) -> Option<Rgb> {
        self.theme_colors
            .iter()
            .find(|(k, _)| k == scheme_name)
            .map(|(_, rgb)| *rgb)
    }
}

// ============================================================================
// DADS CSS custom properties importer (`@digital-go-jp/design-tokens` v2.1.0)
// ============================================================================

/// Parses CSS custom property declarations (`--name: value;`) from a CSS stylesheet,
/// stripping block comments and resolving `var(--ref)` aliases.
pub fn parse_css_custom_properties(css: &str) -> BTreeMap<String, String> {
    let stripped = strip_css_comments(css);
    let mut raw: BTreeMap<String, String> = BTreeMap::new();

    // Scan for `--<name> : <value> ;` (or until `}`) across the stylesheet.
    let bytes = stripped.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'-' && bytes.get(i + 1) == Some(&b'-') {
            // Ensure this is a declaration start (not inside a value or `var(--...)`).
            let prev = stripped[..i].chars().next_back();
            let valid_start = prev.is_none_or(|c| c.is_whitespace() || matches!(c, '{' | ';' | '}'));
            if valid_start {
                let name_start = i;
                i += 2;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'-' | b'_'))
                {
                    i += 1;
                }
                let name = &stripped[name_start..i];
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                if i < bytes.len() && bytes[i] == b':' {
                    i += 1;
                    let val_start = i;
                    let mut in_quote: Option<u8> = None;
                    let mut paren_depth: usize = 0;
                    while i < bytes.len() {
                        let b = bytes[i];
                        if let Some(q) = in_quote {
                            if b == b'\\' {
                                i = (i + 2).min(bytes.len());
                                continue;
                            }
                            if b == q {
                                in_quote = None;
                            }
                            i += 1;
                            continue;
                        }
                        match b {
                            b'"' | b'\'' => {
                                in_quote = Some(b);
                                i += 1;
                            }
                            b'(' => {
                                paren_depth += 1;
                                i += 1;
                            }
                            b')' => {
                                paren_depth = paren_depth.saturating_sub(1);
                                i += 1;
                            }
                            b';' | b'}' if paren_depth == 0 => break,
                            _ => i += 1,
                        }
                    }
                    let val = stripped[val_start..i]
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ");
                    if !name.is_empty() && !val.is_empty() {
                        raw.insert(name.to_string(), val);
                    }
                    continue;
                }
            }
        }
        i += 1;
    }

    // Resolve `var(--other[, fallback])` references (up to 16 hops to prevent cycles).
    let mut resolved = BTreeMap::new();
    for (k, v) in &raw {
        resolved.insert(k.clone(), resolve_css_vars(v, &raw, 0));
    }
    resolved
}

fn strip_css_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        if let Some(end) = after.find("*/") {
            rest = &after[end + 2..];
        } else {
            return out;
        }
    }
    out.push_str(rest);
    out
}

fn resolve_css_vars(value: &str, vars: &BTreeMap<String, String>, depth: usize) -> String {
    if depth >= 16 || !value.contains("var(") {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(pos) = rest.find("var(") {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 4..];
        let mut paren = 1usize;
        let mut end = None;
        for (idx, ch) in after.char_indices() {
            match ch {
                '(' => paren += 1,
                ')' => {
                    paren -= 1;
                    if paren == 0 {
                        end = Some(idx);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end_idx) = end else {
            out.push_str(&rest[pos..]);
            return out;
        };
        let inner = after[..end_idx].trim();
        let (target, fallback) = match inner.split_once(',') {
            Some((t, fb)) => (t.trim(), Some(fb.trim())),
            None => (inner, None),
        };
        if let Some(found) = vars.get(target) {
            out.push_str(&resolve_css_vars(found, vars, depth + 1));
        } else if let Some(fb) = fallback {
            out.push_str(&resolve_css_vars(fb, vars, depth + 1));
        } else {
            out.push_str(&rest[pos..pos + 4 + end_idx + 1]);
        }
        rest = &after[end_idx + 1..];
    }
    out.push_str(rest);
    out
}

/// Imports a Digital Agency Design System (DADS) CSS custom-properties stylesheet
/// (`tokens.css` / `tokens-simple.css` from `@digital-go-jp/design-tokens` v2.1.0) into [`DesignTokens`].
pub fn import_dads_css(css: &str) -> Result<DesignTokens, Error> {
    let props = parse_css_custom_properties(css);
    if props.is_empty() {
        return Err(Error::new(
            "CSS カスタムプロパティ (--token: value;) が見つかりません",
        ));
    }

    let pick_color = |keys: &[&str]| -> Option<String> {
        keys.iter().find_map(|k| {
            let val = props.get(*k)?;
            let rgb = parse_hex_color(val)?;
            Some(format_hex_rgb(rgb))
        })
    };

    let primary = pick_color(&[
        "--color-semantic-accent-primary",
        "--color-primitive-blue-900",
        "--color-primitive-blue-800",
        "--color-primitive-blue-700",
        "--color-primitive-blue-600",
    ]);
    let secondary = pick_color(&[
        "--color-semantic-accent-secondary",
        "--color-primitive-blue-1000",
        "--color-primitive-blue-1100",
        "--color-primitive-blue-1200",
        "--color-neutral-solid-gray-700",
        "--color-neutral-solid-gray-600",
    ]);
    let surface = pick_color(&[
        "--color-semantic-background-base",
        "--color-neutral-white",
    ]);
    let surface_subtle = pick_color(&[
        "--color-semantic-background-subtle",
        "--color-primitive-blue-50",
        "--color-neutral-solid-gray-50",
    ]);
    let text = pick_color(&[
        "--color-semantic-text-body",
        "--color-neutral-solid-gray-900",
        "--color-neutral-solid-gray-800",
        "--color-neutral-black",
    ]);
    let text_muted = pick_color(&[
        "--color-semantic-text-description",
        "--color-neutral-solid-gray-700",
        "--color-neutral-solid-gray-600",
        "--color-neutral-solid-gray-536",
    ]);
    let link = pick_color(&[
        "--color-semantic-link-default",
        "--color-primitive-blue-900",
        "--color-primitive-blue-1000",
    ])
    .or_else(|| primary.clone());
    let visited_link = pick_color(&[
        "--color-semantic-link-visited",
        "--color-primitive-purple-900",
        "--color-primitive-purple-800",
        "--color-primitive-magenta-900",
    ]);

    // Build a high-contrast 6-colour accent / chart sequence from DADS primitive hues.
    let accent_candidates: [&[&str]; 6] = [
        &[
            "--color-semantic-accent-primary",
            "--color-primitive-blue-900",
            "--color-primitive-blue-800",
        ],
        &[
            "--color-primitive-green-800",
            "--color-primitive-green-700",
            "--color-primitive-sea-800",
            "--color-primitive-sea-700",
        ],
        &[
            "--color-primitive-red-800",
            "--color-primitive-red-700",
            "--color-primitive-magenta-800",
        ],
        &[
            "--color-primitive-orange-800",
            "--color-primitive-orange-700",
            "--color-primitive-yellow-900",
            "--color-primitive-yellow-800",
        ],
        &[
            "--color-primitive-cyan-800",
            "--color-primitive-cyan-700",
            "--color-primitive-light-blue-800",
            "--color-primitive-blue-600",
        ],
        &[
            "--color-primitive-purple-800",
            "--color-primitive-purple-700",
            "--color-primitive-lime-800",
        ],
    ];
    let mut accents = Vec::new();
    for group in accent_candidates {
        if let Some(hex) = pick_color(group)
            && !accents.contains(&hex)
        {
            accents.push(hex);
        }
    }

    // Typography
    let font_family = props
        .get("--font-family-sans")
        .and_then(|v| primary_css_font_family(v));
    let heading_font_family = props
        .get("--font-family-heading")
        .and_then(|v| primary_css_font_family(v))
        .or_else(|| font_family.clone());
    let east_asian_font = font_family.clone();
    let mono_font_family = props
        .get("--font-family-mono")
        .and_then(|v| primary_css_font_family(v));

    let pick_pt = |keys: &[&str]| -> Option<f64> {
        keys.iter()
            .find_map(|k| props.get(*k).and_then(|v| parse_size_pt(v)))
    };

    let sizes = TypeScaleTokens {
        body: pick_pt(&["--font-size-16", "--font-size-14"]),
        h1: pick_pt(&["--font-size-32", "--font-size-36", "--font-size-28"]),
        h2: pick_pt(&["--font-size-24", "--font-size-28", "--font-size-22"]),
        h3: pick_pt(&["--font-size-20", "--font-size-22", "--font-size-18"]),
        h4: pick_pt(&["--font-size-18", "--font-size-16"]),
        h5: pick_pt(&["--font-size-16", "--font-size-14"]),
        h6: pick_pt(&["--font-size-14", "--font-size-16"]),
        slide_cover_title: pick_pt(&["--font-size-57", "--font-size-45", "--font-size-48", "--font-size-36"]),
        slide_title: pick_pt(&["--font-size-36", "--font-size-32", "--font-size-28"]),
        slide_body: pick_pt(&["--font-size-24", "--font-size-20", "--font-size-18", "--font-size-16"]),
    };

    let line_height = ["--line-height-150", "--line-height-170", "--line-height-140"]
        .iter()
        .find_map(|k| props.get(*k)?.trim().parse::<f64>().ok());

    // Spacing tokens (`--spacing-*` if present)
    let mut scale = BTreeMap::new();
    for (k, v) in &props {
        if let Some(suffix) = k.strip_prefix("--spacing-")
            && let Some(pt) = parse_size_pt(v)
        {
            scale.insert(suffix.to_string(), pt);
        }
    }

    let has_any_recognized = primary.is_some()
        || surface.is_some()
        || surface_subtle.is_some()
        || text.is_some()
        || font_family.is_some()
        || sizes.body.is_some()
        || sizes.h1.is_some();
    if !has_any_recognized {
        return Err(Error::new(
            "DADS の色またはタイポグラフィトークンが CSS から見つかりませんでした",
        ));
    }

    let chart = accents.clone();
    Ok(DesignTokens {
        name: Some("dads".to_string()),
        source: Some(TokenSourceMeta {
            repository: Some("digital-go-jp/design-tokens".to_string()),
            package: Some("@digital-go-jp/design-tokens".to_string()),
            version: Some("2.1.0".to_string()),
            license: Some("MIT".to_string()),
            note: Some("Imported from DADS CSS custom properties".to_string()),
        }),
        colors: ColorTokens {
            primary,
            secondary,
            surface,
            surface_subtle,
            text,
            text_muted,
            link,
            visited_link,
            accents,
            chart,
        },
        typography: TypographyTokens {
            font_family,
            heading_font_family,
            east_asian_font,
            mono_font_family,
            sizes,
            line_height,
        },
        spacing: SpacingTokens {
            paragraph_after: scale.get("2").copied().or(Some(6.0)),
            heading_before: scale.get("4").copied().or(Some(14.0)),
            heading_after: scale.get("2").copied().or(Some(6.0)),
            scale,
        },
        slide_size: None,
    })
}

// ============================================================================
// Parsing helpers
// ============================================================================

/// Parses `#RRGGBB`, `RRGGBB`, or `#RGB` hex colours into [`Rgb`].
pub fn parse_hex_color(raw: &str) -> Option<Rgb> {
    let hex = raw.trim().strip_prefix('#').unwrap_or(raw.trim());
    match hex.len() {
        6 => u32::from_str_radix(hex, 16).ok().map(Rgb),
        3 => {
            let mut chars = hex.chars();
            let r = chars.next()?.to_digit(16)?;
            let g = chars.next()?.to_digit(16)?;
            let b = chars.next()?.to_digit(16)?;
            Some(Rgb(((r * 17) << 16) | ((g * 17) << 8) | (b * 17)))
        }
        _ => None,
    }
}

fn format_hex_rgb(rgb: Rgb) -> String {
    format!("#{:06X}", rgb.0 & 0xFFFFFF)
}

fn primary_css_font_family(value: &str) -> Option<String> {
    let first = value.split(',').next()?.trim();
    let unquoted = first.trim_matches(|c| c == '"' || c == '\'').trim();
    (!unquoted.is_empty()).then(|| unquoted.to_string())
}

/// Converts a CSS or YAML size token into typographic points (`pt`):
/// - plain number or `<n>pt`: points as-is
/// - `<n>px`: `n * 0.75` (CSS standard 96 dpi = 72 pt/in)
/// - `<n>rem` / `<n>em`: `n * 12.0` (16px root = 12pt)
/// - `<n>mm`: `n * (72.0 / 25.4)`
pub fn parse_size_pt(raw: &str) -> Option<f64> {
    let s = raw.trim();
    let val = if let Some(n) = s.strip_suffix("pt") {
        n.trim().parse::<f64>().ok()?
    } else if let Some(n) = s.strip_suffix("px") {
        n.trim().parse::<f64>().ok()? * 0.75
    } else if let Some(n) = s.strip_suffix("rem") {
        n.trim().parse::<f64>().ok()? * 12.0
    } else if let Some(n) = s.strip_suffix("em") {
        n.trim().parse::<f64>().ok()? * 12.0
    } else if let Some(n) = s.strip_suffix("mm") {
        n.trim().parse::<f64>().ok()? * (72.0 / 25.4)
    } else {
        s.parse::<f64>().ok()?
    };
    val.is_finite().then_some(val)
}

fn parse_length_emu(raw: &str) -> Option<Emu> {
    let s = raw.trim();
    if let Some(v) = s.strip_suffix("mm") {
        v.trim().parse::<f64>().ok().map(|n| (n * EMU_PER_MM).round() as Emu)
    } else if let Some(v) = s.strip_suffix("cm") {
        v.trim().parse::<f64>().ok().map(|n| (n * 10.0 * EMU_PER_MM).round() as Emu)
    } else if let Some(v) = s.strip_suffix("in") {
        v.trim().parse::<f64>().ok().map(|n| (n * EMU_PER_INCH as f64).round() as Emu)
    } else if let Some(v) = s.strip_suffix("pt") {
        v.trim().parse::<f64>().ok().map(|n| (n * EMU_PER_PT as f64).round() as Emu)
    } else if let Some(v) = s.strip_suffix("px") {
        v.trim().parse::<f64>().ok().map(|n| (n * 9_525.0).round() as Emu)
    } else if let Some(v) = s.strip_suffix("emu") {
        v.trim().parse::<i64>().ok()
    } else {
        s.parse::<i64>().ok()
    }
}

fn parse_slide_size_token(sz: &SlideSizeToken) -> Result<EmuSize, Error> {
    if let (Some(w_raw), Some(h_raw)) = (&sz.width, &sz.height) {
        let w = parse_length_emu(w_raw).ok_or_else(|| {
            Error::new(format!("tokens.yaml の slide_size.width が不正です: '{w_raw}'"))
        })?;
        let h = parse_length_emu(h_raw).ok_or_else(|| {
            Error::new(format!("tokens.yaml の slide_size.height が不正です: '{h_raw}'"))
        })?;
        if !(1..=i64::from(i32::MAX)).contains(&w) || !(1..=i64::from(i32::MAX)).contains(&h) {
            return Err(Error::new("tokens.yaml の slide_size が有効範囲外です"));
        }
        return Ok(EmuSize::new(w, h));
    }
    if sz.width.is_some() || sz.height.is_some() {
        return Err(Error::new(
            "tokens.yaml の slide_size は width と height の両方を指定してください",
        ));
    }
    if let Some(preset) = &sz.preset {
        return match preset.trim().to_ascii_lowercase().as_str() {
            "16:9" | "widescreen" => Ok(EmuSize::new(12_192_000, 6_858_000)),
            "4:3" | "standard" => Ok(EmuSize::new(9_144_000, 6_858_000)),
            other => Err(Error::new(format!(
                "tokens.yaml の slide_size.preset が未対応です: '{other}'"
            ))),
        };
    }
    Err(Error::new(
        "tokens.yaml の slide_size には preset または width/height を指定してください",
    ))
}

fn de_opt_pt<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt = Option::<serde_json::Value>::deserialize(deserializer)?;
    match opt {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Number(n)) => n
            .as_f64()
            .map(Some)
            .ok_or_else(|| serde::de::Error::custom("invalid numeric size")),
        Some(serde_json::Value::String(s)) => parse_size_pt(&s)
            .map(Some)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid size value: '{s}'"))),
        Some(other) => Err(serde::de::Error::custom(format!(
            "expected number or size string, got {other}"
        ))),
    }
}

fn de_pt_map<'de, D>(deserializer: D) -> Result<BTreeMap<String, f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = BTreeMap::<String, serde_json::Value>::deserialize(deserializer)?;
    let mut out = BTreeMap::new();
    for (k, v) in raw {
        let pt = match v {
            serde_json::Value::Number(n) => n
                .as_f64()
                .ok_or_else(|| serde::de::Error::custom(format!("invalid numeric spacing for '{k}'")))?,
            serde_json::Value::String(s) => parse_size_pt(&s)
                .ok_or_else(|| serde::de::Error::custom(format!("invalid spacing '{s}' for '{k}'")))?,
            other => {
                return Err(serde::de::Error::custom(format!(
                    "expected number or size string for '{k}', got {other}"
                )));
            }
        };
        out.insert(k, pt);
    }
    Ok(out)
}

fn de_opt_string_or_number<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt = Option::<serde_json::Value>::deserialize(deserializer)?;
    match opt {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(s)) => Ok(Some(s)),
        Some(serde_json::Value::Number(n)) => Ok(Some(n.to_string())),
        Some(other) => Err(serde::de::Error::custom(format!(
            "expected string or number, got {other}"
        ))),
    }
}

fn yaml_str(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn fmt_num(v: f64) -> String {
    if (v - v.round()).abs() < 1e-6 {
        format!("{}", v.round() as i64)
    } else {
        let s = format!("{v:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_DADS_CSS: &str = r#"
/* Excerpt from @digital-go-jp/design-tokens v2.1.0 (MIT) */
:root {
  --color-primitive-blue-50: #e8f1fe;
  --color-primitive-blue-900: #0017c1;
  --color-primitive-blue-1000: #00118f;
  --color-primitive-green-800: #197a4b;
  --color-primitive-red-800: #ec0000;
  --color-primitive-orange-800: #c74700;
  --color-primitive-cyan-800: #006f83;
  --color-primitive-purple-800: #6f2dbd;
  --color-neutral-white: #ffffff;
  --color-neutral-solid-gray-900: #1a1a1c;
  --color-semantic-accent-primary: var(--color-primitive-blue-900);
  --font-family-sans: "Noto Sans JP", -apple-system, BlinkMacSystemFont, sans-serif;
  --font-family-mono: "Noto Sans Mono", monospace;
  --font-size-16: 1rem;
  --font-size-20: 1.25rem;
  --font-size-24: 1.5rem;
  --font-size-32: 2rem;
  --font-size-36: 2.25rem;
  --font-size-57: 3.5625rem;
  --line-height-150: 1.5;
  --spacing-2: 8px;
  --spacing-4: 16px;
}
"#;

    #[test]
    fn imports_dads_css_and_maps_to_theme() {
        let tokens = import_dads_css(SAMPLE_DADS_CSS).expect("import dads css");
        assert_eq!(tokens.colors.primary.as_deref(), Some("#0017C1"));
        assert_eq!(tokens.colors.surface_subtle.as_deref(), Some("#E8F1FE"));
        assert_eq!(tokens.typography.font_family.as_deref(), Some("Noto Sans JP"));
        assert_eq!(tokens.typography.sizes.body, Some(12.0));
        assert_eq!(tokens.typography.sizes.h1, Some(24.0));
        assert_eq!(tokens.typography.sizes.slide_title, Some(27.0));
        assert_eq!(tokens.typography.sizes.slide_cover_title, Some(42.75));

        let theme = tokens.to_theme().expect("to_theme");
        assert_eq!(theme.color("accent1"), Some(Rgb(0x0017C1)));
        assert_eq!(theme.color("lt2"), Some(Rgb(0xE8F1FE)));
        assert_eq!(theme.color("dk1"), Some(Rgb(0x1A1A1C)));
        assert_eq!(theme.color("lt1"), Some(Rgb(0xFFFFFF)));
        assert_eq!(theme.color("hlink"), Some(Rgb(0x0017C1)));
        assert_eq!(theme.chart_colors.len(), 6);
        assert_eq!(theme.chart_colors[0], Rgb(0x0017C1));
        assert_eq!(theme.chart_colors[1], Rgb(0x197A4B));
        assert_eq!(theme.theme_fonts.minor.latin.as_deref(), Some("Noto Sans JP"));
        assert_eq!(theme.theme_fonts.major.east_asian.as_deref(), Some("Noto Sans JP"));
        assert_eq!(theme.body_size_hp, 1200);
        assert_eq!(theme.heading_sizes_hp[0], 2400);
        assert_eq!(theme.slide_title_hp, 2700);

        // Round-trip through generated YAML.
        let yaml = tokens.to_yaml();
        let reparsed = DesignTokens::from_yaml(&yaml).expect("reparse yaml");
        assert_eq!(reparsed.to_theme().unwrap(), theme);
    }

    #[test]
    fn parses_tokens_layouts_styles_and_slide_size() {
        let tokens = DesignTokens::from_yaml(
            r##"
name: test-ds
colors:
  primary: "#0017C1"
  surface_subtle: "#E8F1FE"
typography:
  font_family: "Noto Sans JP"
  sizes:
    body: "14px"
    h1: "28pt"
    slide_title: "32pt"
slide_size:
  preset: "4:3"
"##,
        )
        .expect("parse tokens");
        let theme = tokens.to_theme().unwrap();
        assert_eq!(theme.body_size_hp, 1050);
        assert_eq!(theme.heading_sizes_hp[0], 2800);
        assert_eq!(theme.slide_title_hp, 3200);
        assert_eq!(theme.slide_size, Some(EmuSize::new(9_144_000, 6_858_000)));

        let layouts = LayoutsConfig::from_yaml(
            r#"
layouts:
  - name: title
    master_layout: "タイトル スライド"
    role: cover
  - name: content
    master_layout: "タイトルとコンテンツ"
"#,
        )
        .expect("parse layouts");
        assert_eq!(layouts.layouts.len(), 2);
        assert_eq!(layouts.layouts[0].name, "title");

        let styles = StylesConfig::from_yaml(
            r#"
styles:
  heading_1:
    docx_style: "Heading1"
    based_on: "Normal"
"#,
        )
        .expect("parse styles");
        assert_eq!(
            styles.styles.get("heading_1").and_then(|s| s.docx_style.as_deref()),
            Some("Heading1")
        );
    }

    #[test]
    fn rejects_invalid_hex_and_invalid_names() {
        assert!(DesignTokens::from_yaml("colors:\n  primary: \"not-a-color\"\n").is_err());
        assert!(validate_name("").is_err());
        assert!(validate_name("../escape").is_err());
        assert!(validate_name("a/b").is_err());
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn resolves_project_before_system_and_errors_with_searched_locations() {
        let tmp = tempfile::tempdir().unwrap();
        let project_dir = tmp.path().join("project");
        let system_dir = tmp.path().join("system-data/ibisOffice/DesignSystems");

        let proj_ds = project_dir.join("DesignSystems/brand");
        let sys_ds = system_dir.join("brand");
        let sys_only = system_dir.join("shared");
        std::fs::create_dir_all(&proj_ds).unwrap();
        std::fs::create_dir_all(&sys_ds).unwrap();
        std::fs::create_dir_all(&sys_only).unwrap();

        std::fs::write(
            proj_ds.join("tokens.yaml"),
            "colors:\n  primary: \"#0017C1\"\n",
        )
        .unwrap();
        std::fs::write(
            sys_ds.join("tokens.yaml"),
            "colors:\n  primary: \"#FF0000\"\n",
        )
        .unwrap();
        std::fs::write(
            sys_only.join("tokens.yaml"),
            "colors:\n  primary: \"#00825A\"\n",
        )
        .unwrap();
        // Optional base files are allowed to exist.
        std::fs::write(proj_ds.join("base.pptx"), b"placeholder").unwrap();
        std::fs::write(proj_ds.join("base.docx"), b"placeholder").unwrap();

        // Project wins over system when both exist.
        let loaded = load("brand", Some(&project_dir), Some(&system_dir)).unwrap();
        assert_eq!(loaded.scope, DesignSystemScope::Project);
        assert_eq!(loaded.theme().unwrap().primary, Rgb(0x0017C1));
        assert!(loaded.base_pptx.is_some());
        assert!(loaded.base_docx.is_some());

        // Falls through to system when not in project.
        let shared = load("shared", Some(&project_dir), Some(&system_dir)).unwrap();
        assert_eq!(shared.scope, DesignSystemScope::System);
        assert_eq!(shared.theme().unwrap().primary, Rgb(0x00825A));

        // Listing merges both with project taking precedence.
        let listed = list(Some(&project_dir), Some(&system_dir));
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].name, "brand");
        assert_eq!(listed[0].scope, DesignSystemScope::Project);
        assert_eq!(listed[1].name, "shared");
        assert_eq!(listed[1].scope, DesignSystemScope::System);

        // Missing design system errors clearly with both searched paths (never falls back).
        let err = load("missing-theme", Some(&project_dir), Some(&system_dir))
            .unwrap_err()
            .to_string();
        assert!(err.contains("missing-theme"), "{err}");
        assert!(
            err.contains(&project_dir.join("DesignSystems/missing-theme").display().to_string()),
            "{err}"
        );
        assert!(
            err.contains(&system_dir.join("missing-theme").display().to_string()),
            "{err}"
        );
    }
}
