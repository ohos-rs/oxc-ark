// Portions of this file are derived from Oxc's oxlint implementation.
// Copyright (c) Oxc project contributors.
// Licensed under the MIT License. See https://github.com/oxc-project/oxc/blob/main/LICENSE.

use oxc_diagnostics::{
    Error, Severity,
    reporter::{DiagnosticReporter, DiagnosticResult, Info},
};
use rustc_hash::FxHashMap;

use super::{InternalFormatter, xml_utils::xml_escape};

#[derive(Default)]
pub struct JUnitOutputFormatter;

impl InternalFormatter for JUnitOutputFormatter {
    fn get_diagnostic_reporter(&self) -> Box<dyn DiagnosticReporter> {
        Box::new(JUnitReporter::default())
    }
}

#[derive(Default)]
struct JUnitReporter {
    diagnostics: Vec<Error>,
}

impl DiagnosticReporter for JUnitReporter {
    fn finish(&mut self, _: &DiagnosticResult) -> Option<String> {
        Some(format_junit(&self.diagnostics))
    }

    fn render_error(&mut self, error: Error) -> Option<String> {
        self.diagnostics.push(error);
        None
    }
}

fn format_junit(diagnostics: &[Error]) -> String {
    let mut grouped: FxHashMap<String, Vec<&Error>> = FxHashMap::default();

    for diagnostic in diagnostics {
        let info = Info::new(diagnostic);
        grouped.entry(info.filename).or_default().push(diagnostic);
    }

    let mut filenames: Vec<_> = grouped.keys().cloned().collect();
    filenames.sort();

    let mut total_errors = 0;
    let mut total_warnings = 0;
    let mut test_suites = Vec::new();

    for filename in filenames {
        let diagnostics = grouped.get(&filename).expect("filename collected from map");
        let mut test_cases = String::new();
        let mut error = 0;
        let mut warning = 0;

        for diagnostic in diagnostics {
            let rule = diagnostic
                .code()
                .map_or_else(String::new, |code| code.to_string());
            let Info { message, start, .. } = Info::new(diagnostic);

            let severity = if diagnostic.severity() == Some(Severity::Error) {
                total_errors += 1;
                error += 1;
                "error"
            } else {
                total_warnings += 1;
                warning += 1;
                "failure"
            };
            let description = format!(
                "line {}, column {}, {}",
                start.line,
                start.column,
                xml_escape(&message)
            );

            let status = format!(
                "            <{} message=\"{}\">{}</{}>",
                severity,
                xml_escape(&message),
                description,
                severity
            );
            let test_case =
                format!("\n        <testcase name=\"{rule}\">\n{status}\n        </testcase>");
            test_cases.push_str(&test_case);
        }
        test_suites.push(format!(
            "    <testsuite name=\"{}\" tests=\"{}\" disabled=\"0\" errors=\"{}\" failures=\"{}\">{}\n    </testsuite>",
            filename,
            diagnostics.len(),
            error,
            warning,
            test_cases
        ));
    }
    let test_suites = format!(
        "<testsuites name=\"Oxlint\" tests=\"{}\" failures=\"{}\" errors=\"{}\">\n{}\n</testsuites>\n",
        total_errors + total_warnings,
        total_warnings,
        total_errors,
        test_suites.join("\n")
    );

    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{test_suites}")
}
