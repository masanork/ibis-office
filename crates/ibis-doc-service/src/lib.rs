// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! The headless document service (GitHub issue #159, `tasks/doc/design/2026-11-17-159-mcp.md`): files are opened into [`Service`], read and
//! edited through tools by name (`docx_get_text`, `xlsx_set_cells`, ...), and saved. The tools are listed once, in [`catalog`]; the MCP server and
//! the assistant (#231) are thin doors over it. No window, no GPUI: the model (`ibis-doc-model`), the readers and writers (`ibis-formats`) and the
//! calculation (`ibis-calc`) only.

mod catalog;
mod clock;
mod a1;
mod edit;
mod error;
mod git_read;
mod paths;
mod picture_file;
mod service;
mod tools_app;
mod tools_extract;
mod tools_docx;
mod tools_docx_layout;
mod tools_docx_more;
mod tools_docx_objects;
mod tools_docx_read;
mod tools_docx_set;
mod tools_docx_struct;
mod tools_pptx;
mod tools_pptx_b;
mod tools_xlsx;
mod tools_chart;
mod tools_pivot;
mod tools_wallet;
mod tools_prefill;
mod wallet_dcql;
mod wallet_direct_post;
pub use tools_prefill::{
    review_prefill, reviewed_prefill_command, reviewed_prefill_command_with_options,
};
pub use tools_wallet::run_cli as compare_presentation;
mod tools_xlsx_read;
mod tools_xlsx_set;
mod tools_xlsx_struct;
mod tools_xlsx_chart;

pub use catalog::{Access, Tool, catalog, document_tools};
pub use error::ToolError;
pub use service::{DraftToolContext, Limits, Service, run_draft_tool};

pub use tools_xlsx::WorkbookToolContext;

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
mod tools_sqlite;
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub use tools_sqlite::{DATASET_EXTENSIONS, DESKTOP_TEXT_DATASET_LIMIT, JSON_LINES_RANGE_SQL, ReportParameterControl, ReportParameterKind, SqliteReportBatch, SqliteTableExport, report_parameter_controls, batch_query_report_definitions, is_sqlite, prepare_selected_datasets_export, prepare_selected_json_lines_preview, prepare_selected_query_refresh, prepare_selected_report_batch, query_report_definition, prepare_selected_sqlite_export, run_selected_datasets_tool, run_selected_sqlite_tool, sqlite_tools, trial_selected_report_checks};
