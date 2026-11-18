// Portions of this file are derived from Oxc's oxfmt implementation.
// Copyright (c) Oxc project contributors.
// Licensed under the MIT License. See https://github.com/oxc-project/oxc/blob/main/LICENSE.

use std::path::{Path, PathBuf};

use oxc_formatter::JsFormatOptions as FormatOptions;
use oxc_formatter_json::JsonFormatOptions;
use oxc_toml::Options as TomlFormatterOptions;
use serde_json::Value;

/// Resolve config file path from cwd and optional explicit path.
pub fn resolve_oxfmtrc_path(cwd: &Path, config_path: Option<&Path>) -> Option<PathBuf> {
    // If `--config` is explicitly specified, use that path (aligned with oxfmt)
    if let Some(config_path) = config_path {
        return Some(super::utils::normalize_relative_path(cwd, config_path));
    }

    // If `--config` is not specified, search the nearest config file from cwd upwards
    // Support both `.json` and `.jsonc`, but prefer `.json` if both exist
    cwd.ancestors().find_map(|dir| {
        for filename in [".oxfmtrc.json", ".oxfmtrc.jsonc"] {
            let config_path = dir.join(filename);
            if config_path.exists() {
                return Some(config_path);
            }
        }
        None
    })
}

pub fn resolve_editorconfig_path(cwd: &Path) -> Option<PathBuf> {
    // Search the nearest `.editorconfig` from cwd upwards
    cwd.ancestors()
        .map(|dir| dir.join(".editorconfig"))
        .find(|p| p.exists())
}

// ---

/// Resolved options for each file type.
/// Each variant contains only the options needed for that formatter.
#[derive(Debug)]
pub enum ResolvedOptions {
    Upstream(crate::core::format::FormatPlan),
    /// For JS/TS files formatted by oxc_formatter.
    OxcFormatter {
        format_options: Box<FormatOptions>,
        /// For embedded language formatting (e.g., CSS in template literals)
        external_options: Value,
        insert_final_newline: bool,
    },
    /// For TOML files.
    OxfmtToml {
        toml_options: TomlFormatterOptions,
        insert_final_newline: bool,
    },
    /// For JSON/JSON5/JSONC files.
    OxfmtJson {
        json_options: JsonFormatOptions,
        insert_final_newline: bool,
    },
    /// For non-JS files formatted by external formatter (Prettier).
    #[cfg(feature = "napi")]
    ExternalFormatter {
        external_options: Value,
        insert_final_newline: bool,
    },
    /// For `package.json` files: optionally sorted then formatted.
    #[cfg(feature = "napi")]
    ExternalFormatterPackageJson {
        external_options: Value,
        sort_package_json: bool,
        insert_final_newline: bool,
    },
}

/// The configuration resolver now lives in [`crate::core::config`], alongside
/// associations, nested config scopes and JS config loading.
pub use crate::core::config::ConfigResolver;
