// Derived from ohos-rs/oxc 230deff13c3d0baddb9560ba895ee2b6a3fc4b7e (MIT).
// Copyright (c) Oxc project contributors.

use std::fmt::Write;

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
    // `Info::new` scans the source to resolve the span, so build it exactly once per
    // diagnostic and group by index rather than re-deriving it inside the render loop.
    let infos: Vec<Info> = diagnostics.iter().map(Info::new).collect();

    let mut grouped: FxHashMap<&str, Vec<usize>> = FxHashMap::default();
    for (index, info) in infos.iter().enumerate() {
        grouped
            .entry(info.filename.as_str())
            .or_default()
            .push(index);
    }

    let mut filenames: Vec<&str> = grouped.keys().copied().collect();
    filenames.sort_unstable();

    let mut total_errors = 0;
    let mut total_warnings = 0;
    let mut test_suites = String::new();

    for (suite_index, filename) in filenames.iter().enumerate() {
        let indices = grouped.get(filename).expect("filename collected from map");
        let mut test_cases = String::new();
        let mut error = 0;
        let mut warning = 0;

        for &index in indices {
            let Info {
                message,
                start,
                rule_id,
                ..
            } = &infos[index];
            let rule = rule_id.as_deref().unwrap_or("");

            let severity = if diagnostics[index].severity() == Some(Severity::Error) {
                total_errors += 1;
                error += 1;
                "error"
            } else {
                total_warnings += 1;
                warning += 1;
                "failure"
            };
            let escaped_message = xml_escape(message);

            let _ = write!(
                test_cases,
                "\n        <testcase name=\"{rule}\">\n            <{severity} message=\"{escaped_message}\">line {}, column {}, {escaped_message}</{severity}>\n        </testcase>",
                start.line, start.column,
            );
        }

        if suite_index > 0 {
            test_suites.push('\n');
        }
        let _ = write!(
            test_suites,
            "    <testsuite name=\"{filename}\" tests=\"{}\" disabled=\"0\" errors=\"{error}\" failures=\"{warning}\">{test_cases}\n    </testsuite>",
            indices.len(),
        );
    }

    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites name=\"Oxlint\" tests=\"{}\" failures=\"{total_warnings}\" errors=\"{total_errors}\">\n{test_suites}\n</testsuites>\n",
        total_errors + total_warnings,
    )
}
