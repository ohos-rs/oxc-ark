use std::path::{Path, PathBuf};

use oxc_toml::Options as TomlFormatterOptions;
use serde_json::Value;

use oxc_formatter::{
    FormatOptions,
    oxfmtrc::{OxfmtOptions, Oxfmtrc},
};

use super::FormatFileStrategy;

/// Resolve config file path from cwd and optional explicit path.
pub fn resolve_oxfmtrc_path(cwd: &Path, config_path: Option<&Path>) -> Option<PathBuf> {
    // If `--config` is explicitly specified, use that path
    if let Some(config_path) = config_path {
        return Some(if config_path.is_absolute() {
            config_path.to_path_buf()
        } else {
            cwd.join(config_path)
        });
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
pub enum ResolvedOptions {
    /// For JS/TS files formatted by oxc_formatter.
    OxcFormatter {
        format_options: FormatOptions,
        /// For embedded language formatting (e.g., CSS in template literals)
        external_options: Value,
        insert_final_newline: bool,
    },
    /// For TOML files.
    OxfmtToml {
        toml_options: TomlFormatterOptions,
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

/// Configuration resolver that derives all config values from a single `serde_json::Value`.
pub struct ConfigResolver {
    /// User's raw config as JSON value.
    raw_config: Value,
    /// Cached parsed options after validation.
    cached_options: Option<(FormatOptions, OxfmtOptions, Value)>,
}

impl ConfigResolver {
    /// Create a new resolver from a raw JSON config value.
    pub fn from_value(raw_config: Value) -> Self {
        Self {
            raw_config,
            cached_options: None,
        }
    }

    /// Create a resolver by loading config from a file path.
    ///
    /// # Errors
    /// Returns error if:
    /// - Config file is specified but not found or invalid
    /// - Config file parsing fails
    pub fn from_config_paths(
        _cwd: &Path,
        oxfmtrc_path: Option<&Path>,
        _editorconfig_path: Option<&Path>,
    ) -> Result<Self, String> {
        // Read and parse config file, or use empty JSON if not found
        let json_string = match oxfmtrc_path {
            Some(path) => {
                let mut json_string = super::utils::read_to_string(path)
                    .map_err(|_| format!("Failed to read {}: File not found", path.display()))?;
                // Strip comments (JSONC support)
                json_strip_comments::strip(&mut json_string).map_err(|err| {
                    format!("Failed to strip comments from {}: {err}", path.display())
                })?;
                json_string
            }
            None => "{}".to_string(),
        };

        // Parse as raw JSON value
        let raw_config: Value = serde_json::from_str(&json_string)
            .map_err(|err| format!("Failed to parse config: {err}"))?;

        Ok(Self {
            raw_config,
            cached_options: None,
        })
    }

    /// Validate config and return ignore patterns for file walking.
    ///
    /// Validated options are cached for fast path resolution.
    ///
    /// # Errors
    /// Returns error if config deserialization fails.
    pub fn build_and_validate(&mut self) -> Result<Vec<String>, String> {
        let oxfmtrc: Oxfmtrc = serde_json::from_value(self.raw_config.clone())
            .map_err(|err| format!("Failed to deserialize Oxfmtrc: {err}"))?;

        // If not specified, default options are resolved here
        let (format_options, oxfmt_options) = oxfmtrc
            .into_options()
            .map_err(|err| format!("Failed to parse configuration.\n{err}"))?;

        // Apply our resolved defaults to Prettier options too
        let mut external_options = self.raw_config.clone();
        Oxfmtrc::populate_prettier_config(&format_options, &mut external_options);

        let ignore_patterns_clone = oxfmt_options.ignore_patterns.clone();

        // NOTE: Save cache for fast path
        self.cached_options = Some((format_options, oxfmt_options, external_options));

        Ok(ignore_patterns_clone)
    }

    /// Resolve format options for a specific file.
    pub fn resolve(&self, strategy: &FormatFileStrategy) -> ResolvedOptions {
        let (format_options, oxfmt_options, external_options) = self
            .cached_options
            .clone()
            .expect("`build_and_validate()` must be called before `resolve()`");

        let insert_final_newline = oxfmt_options.insert_final_newline;

        match strategy {
            FormatFileStrategy::OxcFormatter { .. } => ResolvedOptions::OxcFormatter {
                format_options,
                external_options,
                insert_final_newline,
            },
            FormatFileStrategy::OxfmtToml { .. } => ResolvedOptions::OxfmtToml {
                toml_options: build_toml_options(&format_options),
                insert_final_newline,
            },
            #[cfg(feature = "napi")]
            FormatFileStrategy::ExternalFormatter { .. } => ResolvedOptions::ExternalFormatter {
                external_options,
                insert_final_newline,
            },
            #[cfg(feature = "napi")]
            FormatFileStrategy::ExternalFormatterPackageJson { .. } => {
                ResolvedOptions::ExternalFormatterPackageJson {
                    external_options,
                    sort_package_json: oxfmt_options.sort_package_json,
                    insert_final_newline,
                }
            }
            #[cfg(not(feature = "napi"))]
            _ => {
                unreachable!("If `napi` feature is disabled, this should not be passed here")
            }
        }
    }
}

// ---

/// Build `toml` formatter options.
/// The same as `prettier-plugin-toml`.
fn build_toml_options(format_options: &FormatOptions) -> TomlFormatterOptions {
    TomlFormatterOptions {
        column_width: format_options.line_width.value() as usize,
        indent_string: if format_options.indent_style.is_tab() {
            "\t".to_string()
        } else {
            " ".repeat(format_options.indent_width.value() as usize)
        },
        array_trailing_comma: !format_options.trailing_commas.is_none(),
        crlf: format_options.line_ending.is_carriage_return_line_feed(),
        // Align with `oxc_formatter` and Prettier default
        trailing_newline: true,
        ..Default::default()
    }
}
