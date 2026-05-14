// Portions of this file are derived from Oxc's oxlint implementation.
// Copyright (c) Oxc project contributors.
// Licensed under the MIT License. See https://github.com/oxc-project/oxc/blob/main/LICENSE.

mod agent;
mod checkstyle;
mod default;
mod github;
mod gitlab;
mod json;
mod junit;
mod sarif;
mod stylish;
mod unix;
mod xml_utils;

use std::str::FromStr;
use std::time::Duration;

use agent::AgentOutputFormatter;
use checkstyle::CheckStyleOutputFormatter;
use github::GithubOutputFormatter;
use gitlab::GitlabOutputFormatter;
use junit::JUnitOutputFormatter;
use rustc_hash::FxHashSet;
use sarif::SarifOutputFormatter;
use stylish::StylishOutputFormatter;
use unix::UnixOutputFormatter;

use oxc_diagnostics::reporter::DiagnosticReporter;

use crate::output_formatter::{default::DefaultOutputFormatter, json::JsonOutputFormatter};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum OutputFormat {
    Default,
    /// GitHub Check Annotation
    /// <https://docs.github.com/en/actions/using-workflows/workflow-commands-for-github-actions#setting-a-notice-message>
    Github,
    Gitlab,
    Json,
    Unix,
    Agent,
    Checkstyle,
    Stylish,
    JUnit,
    Sarif,
}

impl FromStr for OutputFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "json" => Ok(Self::Json),
            "default" => Ok(Self::Default),
            "unix" => Ok(Self::Unix),
            "agent" => Ok(Self::Agent),
            "checkstyle" => Ok(Self::Checkstyle),
            "github" => Ok(Self::Github),
            "gitlab" => Ok(Self::Gitlab),
            "stylish" => Ok(Self::Stylish),
            "junit" => Ok(Self::JUnit),
            "sarif" => Ok(Self::Sarif),
            _ => Err(format!("'{s}' is not a known format")),
        }
    }
}

/// Some extra lint information, which can be outputted
/// at the end of the command
pub struct LintCommandInfo {
    /// The number of files that were linted.
    pub number_of_files: usize,
    /// The number of lint rules that were run. If the number varies and can't be clearly
    /// computed, then this defaults to None.
    pub number_of_rules: Option<usize>,
    /// The used CPU threads count
    pub threads_count: usize,
    /// Some reporters want to output the duration it took to finished the task
    pub start_time: Duration,
}

impl LintCommandInfo {
    pub(super) fn format_execution_summary(&self) -> String {
        let ms = self.start_time.as_millis();
        let time = if ms < 1000 {
            format!("{ms}ms")
        } else {
            format!("{:.1}s", self.start_time.as_secs_f64())
        };
        let s = if self.number_of_files == 1 { "" } else { "s" };

        if let Some(number_of_rules) = self.number_of_rules {
            format!(
                "Finished in {time} on {} file{s} with {number_of_rules} rules using {} threads.\n",
                self.number_of_files, self.threads_count
            )
        } else {
            format!(
                "Finished in {time} on {} file{s} using {} threads.\n",
                self.number_of_files, self.threads_count
            )
        }
    }
}

/// An Interface for the different output formats.
/// The Formatter is then managed by [`OutputFormatter`].
trait InternalFormatter {
    /// Print all available rules by oxlint
    fn all_rules(&self, _enabled_rules: FxHashSet<&str>) -> Option<String> {
        None
    }

    /// At the end of the Lint command the Formatter can output extra information.
    fn lint_command_info(&self, _lint_command_info: &LintCommandInfo) -> Option<String> {
        None
    }

    /// oxlint words with [`DiagnosticService`](oxc_diagnostics::DiagnosticService),
    /// which uses a own reporter to output to stdout.
    fn get_diagnostic_reporter(&self) -> Box<dyn DiagnosticReporter>;
}

pub struct OutputFormatter {
    internal: Box<dyn InternalFormatter>,
}

impl OutputFormatter {
    pub fn new(format: OutputFormat) -> Self {
        Self {
            internal: Self::get_internal_formatter(format),
        }
    }

    pub fn from_debug_name(format: &str) -> Self {
        let format = match format {
            "Json" => OutputFormat::Json,
            "Checkstyle" => OutputFormat::Checkstyle,
            "Github" => OutputFormat::Github,
            "Gitlab" => OutputFormat::Gitlab,
            "Unix" => OutputFormat::Unix,
            "Agent" => OutputFormat::Agent,
            "Stylish" => OutputFormat::Stylish,
            "JUnit" => OutputFormat::JUnit,
            "Sarif" => OutputFormat::Sarif,
            _ => OutputFormat::Default,
        };
        Self::new(format)
    }

    fn get_internal_formatter(format: OutputFormat) -> Box<dyn InternalFormatter> {
        match format {
            OutputFormat::Json => Box::<JsonOutputFormatter>::default(),
            OutputFormat::Checkstyle => Box::<CheckStyleOutputFormatter>::default(),
            OutputFormat::Github => Box::new(GithubOutputFormatter),
            OutputFormat::Gitlab => Box::<GitlabOutputFormatter>::default(),
            OutputFormat::Unix => Box::<UnixOutputFormatter>::default(),
            OutputFormat::Agent => Box::<AgentOutputFormatter>::default(),
            OutputFormat::Default => Box::new(DefaultOutputFormatter),
            OutputFormat::Stylish => Box::<StylishOutputFormatter>::default(),
            OutputFormat::JUnit => Box::<JUnitOutputFormatter>::default(),
            OutputFormat::Sarif => Box::<SarifOutputFormatter>::default(),
        }
    }

    /// Print all available rules by oxlint
    /// See [`InternalFormatter::all_rules`] for more details.
    pub fn all_rules(&self, enabled_rules: FxHashSet<&str>) -> Option<String> {
        self.internal.all_rules(enabled_rules)
    }

    /// At the end of the Lint command we may output extra information.
    pub fn lint_command_info(&self, lint_command_info: &LintCommandInfo) -> Option<String> {
        self.internal.lint_command_info(lint_command_info)
    }

    /// Returns the [`DiagnosticReporter`] which then will be used by [`DiagnosticService`](oxc_diagnostics::DiagnosticService)
    /// See [`InternalFormatter::get_diagnostic_reporter`] for more details.
    pub fn get_diagnostic_reporter(&self) -> Box<dyn DiagnosticReporter> {
        self.internal.get_diagnostic_reporter()
    }
}
