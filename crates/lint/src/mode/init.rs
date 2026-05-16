// Portions of this file are derived from Oxc's oxlint implementation.
// Copyright (c) Oxc project contributors.
// Licensed under the MIT License. See https://github.com/oxc-project/oxc/blob/main/LICENSE.

use std::{fs, path::Path};

use serde_json::json;

use oxlint::cli::CliRunResult;

use crate::{DEFAULT_OXLINTRC_NAME, runner::print_and_flush_stdout, schema};

pub fn run_init(cwd: &Path, stdout: &mut dyn std::io::Write) -> CliRunResult {
    let mut config = serde_json::Map::new();

    config.insert(
        "$schema".to_string(),
        json!(schema::CONFIGURATION_SCHEMA_PATH),
    );

    config.insert(
        "plugins".to_string(),
        json!(["typescript", "unicorn", "oxc", "arkts"]),
    );
    config.insert("categories".to_string(), json!({ "correctness": "error" }));
    config.insert("rules".to_string(), json!({}));
    config.insert("env".to_string(), json!({ "builtin": true }));

    let configuration = serde_json::to_string_pretty(&serde_json::Value::Object(config)).unwrap();

    if fs::write(cwd.join(DEFAULT_OXLINTRC_NAME), configuration).is_ok() {
        print_and_flush_stdout(stdout, "Configuration file created\n");
        return CliRunResult::ConfigFileInitSucceeded;
    }

    print_and_flush_stdout(stdout, "Failed to create configuration file\n");
    CliRunResult::ConfigFileInitFailed
}
