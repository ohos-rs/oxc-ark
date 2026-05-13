use oxc_linter::{ConfigStoreBuilder, Oxlintrc};
use oxlint::cli::CliRunResult;

use crate::runner::print_and_flush_stdout;

pub fn run_print_config(
    store_builder: &ConfigStoreBuilder,
    oxlintrc: Oxlintrc,
    stdout: &mut dyn std::io::Write,
) -> CliRunResult {
    let config_file = store_builder.resolve_final_config_file(oxlintrc);
    print_and_flush_stdout(stdout, &config_file);
    print_and_flush_stdout(stdout, "\n");

    CliRunResult::PrintConfigResult
}
