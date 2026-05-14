// Portions of this file are derived from Oxc's oxlint implementation.
// Copyright (c) Oxc project contributors.
// Licensed under the MIT License. See https://github.com/oxc-project/oxc/blob/main/LICENSE.

mod init;
mod print_config;
mod rules;

pub use init::run_init;
pub use print_config::run_print_config;
pub use rules::run_rules;
