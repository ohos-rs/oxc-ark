// Portions of this file are derived from Oxc's oxlint implementation.
// Copyright (c) Oxc project contributors.
// Licensed under the MIT License. See https://github.com/oxc-project/oxc/blob/main/LICENSE.

use std::borrow::Cow;

use oxc_diagnostics::{
    Error, Severity,
    reporter::{DiagnosticReporter, DiagnosticResult, Info},
};

use crate::output_formatter::InternalFormatter;

#[derive(Debug, Default)]
pub struct UnixOutputFormatter;

impl InternalFormatter for UnixOutputFormatter {
    fn get_diagnostic_reporter(&self) -> Box<dyn DiagnosticReporter> {
        Box::new(UnixReporter::default())
    }
}

/// Reporter to output diagnostics in a simple one line output.
/// At the end it reports the total numbers of diagnostics.
#[derive(Default)]
struct UnixReporter {
    total: usize,
}

impl DiagnosticReporter for UnixReporter {
    fn finish(&mut self, _: &DiagnosticResult) -> Option<String> {
        let total = self.total;
        if total > 0 {
            return Some(format!(
                "\n{total} problem{}\n",
                if total > 1 { "s" } else { "" }
            ));
        }

        None
    }

    fn supports_minified_file_fallback(&self) -> bool {
        false
    }

    fn render_error(&mut self, error: Error) -> Option<String> {
        self.total += 1;
        Some(format_unix(&error))
    }
}

/// <https://github.com/fregante/eslint-formatters/tree/ae1fd9748596447d1fd09625c33d9e7ba9a3d06d/packages/eslint-formatter-unix>
fn format_unix(diagnostic: &Error) -> String {
    let Info {
        start,
        end: _,
        filename,
        message,
        severity,
        rule_id,
    } = Info::new(diagnostic);
    let severity = match severity {
        Severity::Error => "Error",
        _ => "Warning",
    };
    let rule_id = rule_id.map_or_else(
        || Cow::Borrowed(""),
        |rule_id| Cow::Owned(format!("/{rule_id}")),
    );
    format!(
        "{filename}:{}:{}: {message} [{severity}{rule_id}]\n",
        start.line, start.column
    )
}
