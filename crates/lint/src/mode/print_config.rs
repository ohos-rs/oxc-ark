// Portions of this file are derived from Oxc's oxlint implementation.
// Copyright (c) Oxc project contributors.
// Licensed under the MIT License. See https://github.com/oxc-project/oxc/blob/main/LICENSE.

use oxc_linter::ConfigStore;
use oxlint::cli::CliRunResult;

use crate::runner::print_and_flush_stdout;

pub fn run_print_config(
    config_store: &ConfigStore,
    stdout: &mut dyn std::io::Write,
) -> CliRunResult {
    let config_file = config_store.to_config_file();
    print_and_flush_stdout(stdout, &config_file);
    print_and_flush_stdout(stdout, "\n");

    CliRunResult::PrintConfigResult
}
