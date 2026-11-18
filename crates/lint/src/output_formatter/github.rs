// Derived from ohos-rs/oxc 230deff13c3d0baddb9560ba895ee2b6a3fc4b7e (MIT).
// Copyright (c) Oxc project contributors.

use std::borrow::Cow;

use oxc_diagnostics::{
    Error, Severity,
    reporter::{DiagnosticReporter, DiagnosticResult, Info},
};

use super::default::get_diagnostic_result_output;
use crate::output_formatter::InternalFormatter;

#[derive(Debug)]
pub struct GithubOutputFormatter;

impl InternalFormatter for GithubOutputFormatter {
    fn lint_command_info(&self, lint_command_info: &super::LintCommandInfo) -> Option<String> {
        Some(lint_command_info.format_execution_summary())
    }

    fn get_diagnostic_reporter(&self) -> Box<dyn DiagnosticReporter> {
        Box::new(GithubReporter)
    }
}

/// Formats reports using [GitHub Actions
/// annotations](https://docs.github.com/en/actions/reference/workflow-commands-for-github-actions#setting-an-error-message). Useful for reporting in CI.
struct GithubReporter;

impl DiagnosticReporter for GithubReporter {
    fn finish(&mut self, result: &DiagnosticResult) -> Option<String> {
        Some(get_diagnostic_result_output(result))
    }

    fn supports_minified_file_fallback(&self) -> bool {
        false
    }

    fn render_error(&mut self, error: Error) -> Option<String> {
        Some(format_github(&error))
    }
}

fn format_github(diagnostic: &Error) -> String {
    let Info {
        start,
        end,
        filename,
        message,
        severity,
        rule_id,
    } = Info::new(diagnostic);
    let severity = match severity {
        Severity::Error => "error",
        Severity::Warning | Severity::Advice => "warning",
    };
    let title = rule_id.map_or(Cow::Borrowed("oxlint"), Cow::Owned);

    if filename.is_empty() {
        let severity = match diagnostic.severity() {
            Some(Severity::Error) | None => "error",
            Some(Severity::Warning | Severity::Advice) => "warning",
        };
        let message = diagnostic.to_string();
        let message = escape_data(&message);
        format!("::{severity} title={title}::{message}\n")
    } else {
        // The parameters before `::` only feed the annotations panel, not the log
        // stream, so repeat `file:line:col` in the message text. Same layout as Ruff:
        // https://github.com/astral-sh/ruff/blob/main/crates/ruff_db/src/diagnostic/render/github.rs
        let escaped_filename = escape_property(&filename);
        let filename_data = escape_data(&filename);
        let message = escape_data(&message);
        let (line, col) = (start.line, start.column);
        let (end_line, end_col) = (end.line, end.column);
        format!(
            "::{severity} file={escaped_filename},line={line},endLine={end_line},col={col},endColumn={end_col},title={title}::{filename_data}:{line}:{col}: {message}\n"
        )
    }
}

fn escape_data(value: &str) -> String {
    // Refs:
    // - https://github.com/actions/runner/blob/a4c57f27477077e57545af79851551ff7f5632bd/src/Runner.Common/ActionCommand.cs#L18-L22
    // - https://github.com/actions/toolkit/blob/fe3e7ce9a7f995d29d1fcfd226a32bca407f9dc8/packages/core/src/command.ts#L80-L94
    let mut result = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\r' => result.push_str("%0D"),
            '\n' => result.push_str("%0A"),
            '%' => result.push_str("%25"),
            _ => result.push(c),
        }
    }
    result
}

fn escape_property(value: &str) -> String {
    // Refs:
    // - https://github.com/actions/runner/blob/a4c57f27477077e57545af79851551ff7f5632bd/src/Runner.Common/ActionCommand.cs#L25-L32
    // - https://github.com/actions/toolkit/blob/fe3e7ce9a7f995d29d1fcfd226a32bca407f9dc8/packages/core/src/command.ts#L80-L94
    let mut result = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\r' => result.push_str("%0D"),
            '\n' => result.push_str("%0A"),
            ':' => result.push_str("%3A"),
            ',' => result.push_str("%2C"),
            '%' => result.push_str("%25"),
            _ => result.push(c),
        }
    }
    result
}
