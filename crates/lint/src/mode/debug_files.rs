// Derived from ohos-rs/oxc 230deff13c3d0baddb9560ba895ee2b6a3fc4b7e (MIT).
// Copyright (c) Oxc project contributors.

use std::{io::Write, path::Path};

use cow_utils::CowUtils;

use crate::runner::print_and_flush_stdout;
use oxlint::cli::CliRunResult;

pub fn run_debug_files(
    files_to_lint: impl IntoIterator<Item = impl AsRef<Path>>,
    cwd: &Path,
    stdout: &mut dyn Write,
) -> CliRunResult {
    let mut files = files_to_lint.into_iter().collect::<Vec<_>>();
    files.sort_unstable_by(|a, b| a.as_ref().cmp(b.as_ref()));

    let mut output = String::new();
    for file in files {
        push_cwd_relative_path(&mut output, file.as_ref(), cwd);
        output.push('\n');
    }

    if !output.is_empty() {
        print_and_flush_stdout(stdout, &output);
    }

    CliRunResult::LintSucceeded
}

fn push_cwd_relative_path(output: &mut String, path: &Path, cwd: &Path) {
    let path = path.strip_prefix(cwd).unwrap_or(path);
    output.push_str(&path.to_string_lossy().cow_replace('\\', "/"));
}
