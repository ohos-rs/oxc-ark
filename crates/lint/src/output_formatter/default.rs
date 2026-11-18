// Derived from ohos-rs/oxc 230deff13c3d0baddb9560ba895ee2b6a3fc4b7e (MIT).
// Copyright (c) Oxc project contributors.

use std::{fmt::Write, time::Duration};

use crate::output_formatter::{InternalFormatter, RuleTimings};
use oxc_diagnostics::{
    Error, GraphicalReportHandler,
    reporter::{DiagnosticReporter, DiagnosticResult},
};
use oxc_linter::{RuleTimingRecord, RuleTimingSource, table::RuleTable};
use rustc_hash::FxHashSet;

#[derive(Debug)]
pub struct DefaultOutputFormatter;

impl InternalFormatter for DefaultOutputFormatter {
    fn all_rules(&self, enabled_rules: FxHashSet<(&str, &str)>) -> Option<String> {
        let mut output = String::new();
        let table = RuleTable::default();
        for section in &table.sections {
            output.push_str(&section.render_markdown_table_cli(&enabled_rules));
            output.push('\n');
        }
        output.push_str(format!("Default: {}\n", table.turned_on_by_default_count).as_str());
        output.push_str(format!("Total: {}\n", table.total).as_str());
        Some(output)
    }

    fn lint_command_info(&self, lint_command_info: &super::LintCommandInfo) -> Option<String> {
        let mut output = lint_command_info.format_execution_summary();

        if let Some(rule_timings) = &lint_command_info.rule_timings {
            if !rule_timings.records.is_empty() {
                output.push('\n');
                output.push_str(&format_rule_timing_table(&rule_timings.records));
            }
            if !rule_timings.js_plugin_runtime.is_zero() {
                output.push('\n');
                output.push_str(&format_js_plugin_timing_summary(rule_timings));
            }
        }

        Some(output)
    }

    #[cfg(not(any(test, feature = "testing")))]
    fn get_diagnostic_reporter(&self) -> Box<dyn DiagnosticReporter> {
        Box::new(GraphicalReporter::default())
    }

    #[cfg(any(test, feature = "testing"))]
    fn get_diagnostic_reporter(&self) -> Box<dyn DiagnosticReporter> {
        use crate::output_formatter::default::test_implementation::GraphicalReporterTester;

        Box::new(GraphicalReporterTester::default())
    }
}

fn format_js_plugin_timing_summary(rule_timings: &RuleTimings) -> String {
    let runtime = rule_timings.js_plugin_runtime;
    let rule_callbacks = rule_timings
        .records
        .iter()
        .filter(|record| record.source == RuleTimingSource::JsPlugin)
        .map(|record| record.duration)
        .sum::<Duration>();
    let shared_overhead = runtime.saturating_sub(rule_callbacks);

    format!(
        "JS plugin runtime:\n  Total:           {:>10.3}ms\n  Rule callbacks:  {:>10.3}ms\n  Shared overhead: {:>10.3}ms\n",
        runtime.as_secs_f64() * 1000.0,
        rule_callbacks.as_secs_f64() * 1000.0,
        shared_overhead.as_secs_f64() * 1000.0,
    )
}

fn format_rule_timing_table(rule_timings: &[RuleTimingRecord]) -> String {
    let rule_names = rule_timings
        .iter()
        .map(|record| format!("{}/{}", record.plugin_name, record.rule_name))
        .collect::<Vec<_>>();

    let source_width = rule_timings
        .iter()
        .map(|record| record.source.as_str().len())
        .max()
        .unwrap_or("Source".len())
        .max("Source".len());
    let rule_width = rule_names
        .iter()
        .map(String::len)
        .max()
        .unwrap_or("Rule".len())
        .max("Rule".len());
    let calls_width = rule_timings
        .iter()
        .map(|record| record.calls.to_string().len())
        .max()
        .unwrap_or("Calls".len())
        .max("Calls".len());
    let total_millis = rule_timings
        .iter()
        .map(|record| record.duration.as_secs_f64() * 1000.0)
        .sum::<f64>();

    let mut output = String::new();
    output.push_str("Rule timings:\n");
    writeln!(
        output,
        "{:<rule_width$}  {:>10}  {:>8}  {:>calls_width$}  Source",
        "Rule", "Time (ms)", "Relative", "Calls",
    )
    .unwrap();
    writeln!(
        output,
        "{:-<rule_width$}  {:-<10}  {:-<8}  {:-<calls_width$}  {:-<source_width$}",
        "", "", "", "", "",
    )
    .unwrap();

    for (record, rule_name) in rule_timings.iter().zip(rule_names) {
        let millis = record.duration.as_secs_f64() * 1000.0;
        let relative = if total_millis > 0.0 {
            millis / total_millis * 100.0
        } else {
            0.0
        };
        writeln!(
            output,
            "{:<rule_width$}  {:>10.3}  {:>7.1}%  {:>calls_width$}  {}",
            rule_name,
            millis,
            relative,
            record.calls,
            record.source.as_str(),
        )
        .unwrap();
    }

    output
}

/// Pretty-prints diagnostics. Primarily meant for human-readable output in a terminal.
///
/// See [`GraphicalReportHandler`] for how to configure colors, context lines, etc.
#[cfg_attr(any(test, feature = "testing"), allow(dead_code))]
struct GraphicalReporter {
    handler: GraphicalReportHandler,
}

impl Default for GraphicalReporter {
    fn default() -> Self {
        Self {
            handler: GraphicalReportHandler::new(),
        }
    }
}

impl DiagnosticReporter for GraphicalReporter {
    fn finish(&mut self, result: &DiagnosticResult) -> Option<String> {
        Some(get_diagnostic_result_output(result))
    }

    fn render_error(&mut self, error: Error) -> Option<String> {
        let mut output = String::with_capacity(384);
        self.handler
            .render_report(&mut output, error.as_ref())
            .unwrap();
        Some(output)
    }

    fn render_errors(&mut self, errors: Vec<Error>, emit: &mut dyn FnMut(&str)) {
        let capacity = errors.len().checked_mul(384).unwrap_or(0);
        let mut output = String::with_capacity(capacity);
        self.handler
            .render_reports(
                &mut output,
                errors
                    .iter()
                    .map(|error| error.as_ref() as &dyn oxc_diagnostics::Diagnostic),
            )
            .unwrap();
        emit(&output);
    }

    fn render_errors_until(
        &mut self,
        errors: Vec<Error>,
        keep: &mut dyn FnMut(Option<&str>, &str) -> bool,
    ) {
        self.handler
            .render_reports_until(
                errors
                    .iter()
                    .map(|error| error.as_ref() as &dyn oxc_diagnostics::Diagnostic),
                &mut |diagnostic, rendered| {
                    let source_name = diagnostic.source_code().and_then(|source| source.name());
                    keep(source_name, rendered)
                },
            )
            .unwrap();
    }
}

pub(super) fn get_diagnostic_result_output(result: &DiagnosticResult) -> String {
    let mut output = String::new();

    if result.warnings_count() + result.errors_count() > 0 {
        output.push('\n');
    }

    output.push_str(
        format!(
            "Found {} warning{} and {} error{}.\n",
            result.warnings_count(),
            if result.warnings_count() == 1 {
                ""
            } else {
                "s"
            },
            result.errors_count(),
            if result.errors_count() == 1 { "" } else { "s" },
        )
        .as_str(),
    );

    if result.max_warnings_exceeded() {
        output.push_str(
            format!(
                "Exceeded maximum number of warnings. Found {}.\n",
                result.warnings_count()
            )
            .as_str(),
        );
    }

    output
}

#[cfg(any(test, feature = "testing"))]
mod test_implementation {
    use oxc_diagnostics::{
        Error, GraphicalReportHandler, GraphicalTheme,
        reporter::{DiagnosticReporter, DiagnosticResult, Info},
    };

    use crate::output_formatter::default::get_diagnostic_result_output;

    #[derive(Default)]
    pub struct GraphicalReporterTester {
        diagnostics: Vec<Error>,
    }

    impl DiagnosticReporter for GraphicalReporterTester {
        fn finish(&mut self, result: &DiagnosticResult) -> Option<String> {
            let handler = GraphicalReportHandler::new_themed(GraphicalTheme::none())
                // links print ansi escape codes, which makes snapshots harder to read
                .with_links(false);
            let mut output = String::new();

            self.diagnostics.sort_by_cached_key(|diagnostic| {
                let info = Info::new(diagnostic);
                (
                    info.filename,
                    info.start,
                    info.end,
                    info.rule_id,
                    info.message,
                )
            });

            for diagnostic in &self.diagnostics {
                handler
                    .render_report(&mut output, diagnostic.as_ref())
                    .unwrap();
            }

            output.push_str(&get_diagnostic_result_output(result));

            Some(output)
        }

        fn render_error(&mut self, error: Error) -> Option<String> {
            self.diagnostics.push(error);
            None
        }
    }
}
