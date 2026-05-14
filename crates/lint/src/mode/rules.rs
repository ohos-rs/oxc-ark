// Portions of this file are derived from Oxc's oxlint implementation.
// Copyright (c) Oxc project contributors.
// Licensed under the MIT License. See https://github.com/oxc-project/oxc/blob/main/LICENSE.

use oxc_linter::Config;
use oxlint::cli::CliRunResult;
use rustc_hash::FxHashSet;

use crate::{output_formatter::OutputFormatter, runner::print_and_flush_stdout};

/// If the user requested `--rules`, print a CLI-specific table that
/// includes an "Enabled?" column based on the resolved configuration.
pub fn run_rules(
    lint_config: &Config,
    output_formatter: &OutputFormatter,
    stdout: &mut dyn std::io::Write,
) -> CliRunResult {
    // Build the set of enabled builtin rule names from the resolved config.
    let enabled: FxHashSet<&str> = lint_config
        .rules()
        .iter()
        .map(|(rule, _)| rule.name())
        .collect();

    if let Some(output) = output_formatter.all_rules(enabled) {
        print_and_flush_stdout(stdout, &output);
    }

    CliRunResult::None
}
