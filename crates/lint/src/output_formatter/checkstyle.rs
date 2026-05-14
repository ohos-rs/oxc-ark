// Portions of this file are derived from Oxc's oxlint implementation.
// Copyright (c) Oxc project contributors.
// Licensed under the MIT License. See https://github.com/oxc-project/oxc/blob/main/LICENSE.

use std::borrow::Cow;

use rustc_hash::FxHashMap;

use oxc_diagnostics::{
    Error, Severity,
    reporter::{DiagnosticReporter, DiagnosticResult, Info},
};

use crate::output_formatter::{InternalFormatter, xml_utils::xml_escape};

#[derive(Debug, Default)]
pub struct CheckStyleOutputFormatter;

impl InternalFormatter for CheckStyleOutputFormatter {
    fn get_diagnostic_reporter(&self) -> Box<dyn DiagnosticReporter> {
        Box::new(CheckstyleReporter::default())
    }
}

/// Reporter to output diagnostics in checkstyle format
///
/// Checkstyle Format Documentation: <https://checkstyle.sourceforge.io/>
#[derive(Default)]
struct CheckstyleReporter {
    diagnostics: Vec<Error>,
}

impl DiagnosticReporter for CheckstyleReporter {
    fn finish(&mut self, _: &DiagnosticResult) -> Option<String> {
        Some(format_checkstyle(&self.diagnostics))
    }

    fn render_error(&mut self, error: Error) -> Option<String> {
        self.diagnostics.push(error);
        None
    }
}

fn format_checkstyle(diagnostics: &[Error]) -> String {
    let infos = diagnostics.iter().map(Info::new).collect::<Vec<_>>();
    let mut grouped: FxHashMap<String, Vec<Info>> = FxHashMap::default();
    for info in infos {
        grouped.entry(info.filename.clone()).or_default().push(info);
    }
    let messages = grouped.into_values().map(|infos| {
         let messages = infos
             .iter()
             .fold(String::new(), |mut acc, info| {
                 let Info { start, message, severity, rule_id, .. } = info;
                 let severity = match severity {
                     Severity::Error => "error",
                     _ => "warning",
                 };
                 let message =  xml_escape(message);
                 let source = rule_id.as_ref().map_or(Cow::Borrowed(""), |v| xml_escape(v));
                 let line = format!(r#"<error line="{}" column="{}" severity="{severity}" message="{message}" source="{source}" />"#, start.line, start.column);
                 acc.push_str(&line);
                 acc
             });
         let filename = &infos[0].filename;
         format!(r#"<file name="{filename}">{messages}</file>"#)
     }).collect::<Vec<_>>().join(" ");
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><checkstyle version=\"4.3\">{messages}</checkstyle>\n"
    )
}
